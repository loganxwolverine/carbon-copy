//! Parsing du « mode robot » de `makemkvcon` (option `-r`).
//!
//! En mode robot, chaque ligne de `stdout` a la forme `PREFIXE:champ1,champ2,...`.
//! Les champs texte sont entre guillemets doubles et peuvent contenir des
//! virgules ; on ne peut donc PAS faire un simple `split(',')`. Ce module
//! fournit un mini-parseur CSV tolérant, puis convertit chaque ligne en
//! [`RobotLine`] typée.
//!
//! Préfixes utilisés :
//! - `MSG:code,flags,nb_params,"message","format",params...` : message lisible
//! - `PRGT:code,id,"nom"` : intitulé de l'opération globale en cours
//! - `PRGC:code,id,"nom"` : intitulé de la sous-opération en cours
//! - `PRGV:courant,total,max` : progression (courant = sous-op, total = global)
//! - `DRV:index,visible,enabled,flags,"lecteur","disque","/dev/srX"`
//! - `TCOUNT:n` : nombre de titres
//! - `CINFO:id_attr,code,"valeur"` : attribut du disque
//! - `TINFO:titre,id_attr,code,"valeur"` : attribut d'un titre
//! - `SINFO:titre,flux,id_attr,code,"valeur"` : attribut d'un flux (piste)

/// Une ligne de sortie robot, décodée.
#[derive(Debug, Clone, PartialEq)]
pub enum RobotLine {
    Msg { code: u32, text: String },
    ProgressTitle { name: String },
    ProgressCurrent { name: String },
    ProgressValue { current: u64, total: u64, max: u64 },
    Drive(DriveEntry),
    TitleCount(u32),
    DiscAttr { id: u32, value: String },
    TitleAttr { title: usize, id: u32, value: String },
    StreamAttr { title: usize, stream: usize, id: u32, value: String },
    Unknown(String),
}

/// Entrée `DRV:` : un lecteur optique vu par MakeMKV.
#[derive(Debug, Clone, PartialEq)]
pub struct DriveEntry {
    pub index: u32,
    /// État : 0 = vide/fermé, 1 = vide/ouvert, 2 = disque présent, 3 = chargement,
    /// 256 = emplacement inexistant.
    pub state: u32,
    pub flags: u32,
    pub drive_name: String,
    pub disc_name: String,
    pub device: String,
}

impl DriveEntry {
    pub fn exists(&self) -> bool {
        self.state != 256 && !self.drive_name.is_empty()
    }
}

/// Identifiants d'attributs MakeMKV (`apdefs.h` de makemkv-oss).
pub mod attr {
    pub const TYPE: u32 = 1;
    pub const NAME: u32 = 2;
    pub const LANG_CODE: u32 = 3;
    pub const LANG_NAME: u32 = 4;
    pub const CODEC_SHORT: u32 = 6;
    pub const CODEC_LONG: u32 = 7;
    pub const CHAPTER_COUNT: u32 = 8;
    pub const DURATION: u32 = 9;
    pub const DISK_SIZE: u32 = 10;
    pub const DISK_SIZE_BYTES: u32 = 11;
    pub const SOURCE_FILE: u32 = 16;
    pub const VIDEO_SIZE: u32 = 19;
    pub const STREAM_FLAGS: u32 = 22;
    pub const TREE_INFO: u32 = 30;
    pub const VOLUME_NAME: u32 = 32;
    pub const MKV_FLAGS_TEXT: u32 = 39;
    pub const AUDIO_LAYOUT: u32 = 40;
}

/// Drapeaux de flux (`ap_iaStreamFlags`).
pub mod stream_flags {
    pub const DIRECTORS_COMMENTS: u32 = 1;
    pub const ALT_DIRECTORS_COMMENTS: u32 = 2;
    pub const VISUALLY_IMPAIRED: u32 = 4;
    pub const CORE_AUDIO: u32 = 256;
    pub const SECONDARY_AUDIO: u32 = 512;
    pub const DERIVED_STREAM: u32 = 2048;
    pub const FORCED_SUBTITLES: u32 = 4096;
}

/// Découpe la partie « valeurs » d'une ligne robot en champs.
///
/// Gère : guillemets doubles, virgules à l'intérieur des guillemets,
/// et échappement par antislash (`\"`). Les champs non quotés sont rognés.
pub fn split_fields(s: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if in_quotes => {
                // Échappement : on prend le caractère suivant tel quel.
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            }
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => {
                fields.push(std::mem::take(&mut cur));
            }
            _ => cur.push(c),
        }
    }
    fields.push(cur);
    fields
}

fn num<T: std::str::FromStr + Default>(f: &[String], i: usize) -> T {
    f.get(i).and_then(|v| v.trim().parse().ok()).unwrap_or_default()
}

fn text(f: &[String], i: usize) -> String {
    f.get(i).cloned().unwrap_or_default()
}

/// Décode une ligne brute de `makemkvcon -r`.
pub fn parse_line(line: &str) -> RobotLine {
    let line = line.trim_end_matches(['\r', '\n']);
    let Some((prefix, rest)) = line.split_once(':') else {
        return RobotLine::Unknown(line.to_string());
    };
    let f = split_fields(rest);
    match prefix {
        "MSG" => RobotLine::Msg { code: num(&f, 0), text: text(&f, 3) },
        "PRGT" => RobotLine::ProgressTitle { name: text(&f, 2) },
        "PRGC" => RobotLine::ProgressCurrent { name: text(&f, 2) },
        "PRGV" => RobotLine::ProgressValue {
            current: num(&f, 0),
            total: num(&f, 1),
            max: num(&f, 2),
        },
        "DRV" => RobotLine::Drive(DriveEntry {
            index: num(&f, 0),
            state: num(&f, 1),
            flags: num(&f, 3),
            drive_name: text(&f, 4),
            disc_name: text(&f, 5),
            device: text(&f, 6),
        }),
        "TCOUNT" => RobotLine::TitleCount(num(&f, 0)),
        "CINFO" => RobotLine::DiscAttr { id: num(&f, 0), value: text(&f, 2) },
        "TINFO" => RobotLine::TitleAttr { title: num(&f, 0), id: num(&f, 1), value: text(&f, 3) },
        "SINFO" => RobotLine::StreamAttr {
            title: num(&f, 0),
            stream: num(&f, 1),
            id: num(&f, 2),
            value: text(&f, 4),
        },
        _ => RobotLine::Unknown(line.to_string()),
    }
}

/// Convertit une durée MakeMKV « h:mm:ss » en secondes.
pub fn parse_duration(s: &str) -> u64 {
    s.split(':')
        .map(|p| p.trim().parse::<u64>().unwrap_or(0))
        .fold(0, |acc, v| acc * 60 + v)
}

/// Formate des secondes en « 2 h 05 min ».
pub fn format_duration(secs: u64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    if h > 0 {
        format!("{h} h {m:02} min")
    } else {
        format!("{m} min {:02} s", secs % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_handles_quotes_and_commas() {
        let f = split_fields(r#"0,2,999,1,"BD-RE HL-DT-ST, rev 1.0","MON_FILM","/dev/sr0""#);
        assert_eq!(f.len(), 7);
        assert_eq!(f[4], "BD-RE HL-DT-ST, rev 1.0");
        assert_eq!(f[6], "/dev/sr0");
    }

    #[test]
    fn split_handles_escaped_quotes() {
        let f = split_fields(r#"1,2,"Il a dit \"bonjour\"""#);
        assert_eq!(f[2], r#"Il a dit "bonjour""#);
    }

    #[test]
    fn parse_drv() {
        let l = parse_line(r#"DRV:0,2,999,12,"BD-RE ASUS","THE_MATRIX","/dev/sr0""#);
        match l {
            RobotLine::Drive(d) => {
                assert_eq!(d.state, 2);
                assert_eq!(d.disc_name, "THE_MATRIX");
                assert_eq!(d.device, "/dev/sr0");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn parse_sinfo_and_prgv() {
        assert_eq!(
            parse_line(r#"SINFO:0,3,3,0,"fra""#),
            RobotLine::StreamAttr { title: 0, stream: 3, id: 3, value: "fra".into() }
        );
        assert_eq!(
            parse_line("PRGV:120,32768,65536"),
            RobotLine::ProgressValue { current: 120, total: 32768, max: 65536 }
        );
        assert_eq!(
            parse_line(r#"MSG:5021,260,1,"This application version is too old.","%1",""#),
            RobotLine::Msg { code: 5021, text: "This application version is too old.".into() }
        );
    }

    #[test]
    fn durations() {
        assert_eq!(parse_duration("2:16:22"), 2 * 3600 + 16 * 60 + 22);
        assert_eq!(format_duration(8182), "2 h 16 min");
    }
}
