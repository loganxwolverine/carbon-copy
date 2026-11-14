//! Informations sur le lecteur optique : modèle, firmware, connexion,
//! capacités de lecture / gravure et disque inséré.
//!
//! Sources (aucun accès au lecteur lui-même, donc instantané et sans réveil) :
//! - `/sys/block/srN/device/{vendor,model,rev}` : identité et version du firmware ;
//! - `/run/udev/data/bMAJ:MIN` : capacités (`ID_CDROM_*`), bus, média inséré ;
//! - `/proc/sys/dev/cdrom/info` : vitesse maximale ;
//! - `~/MakeMKV_log.txt` : identifiant SDF de MakeMKV, qui contient la date du
//!   firmware et le numéro de série (`HL-DT-ST_DVDRAM_GSA-T50N_RE06_20090730123456_M000EXEMPLE`).

use std::collections::BTreeSet;
use std::path::Path;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct DriveInfo {
    pub device: String,
    pub vendor: String,
    pub model: String,
    pub firmware: String,
    /// Date du firmware (AAAA-MM-JJ), si MakeMKV l'a relevée.
    pub firmware_date: Option<String>,
    pub serial: Option<String>,
    /// « USB », « SATA »…
    pub bus: Option<String>,
    /// Vitesse maximale (multiple de la vitesse CD).
    pub max_speed: Option<u32>,
    /// Formats lisibles : CD, DVD, Blu-ray…
    pub reads: Vec<&'static str>,
    /// Formats gravables : CD-R, DVD±R DL, BD-RE…
    pub writes: Vec<&'static str>,
    /// Disque actuellement inséré (ex. « DVD+R DL »).
    pub media: Option<String>,
    /// Zone DVD du lecteur (RPC-2), si le lecteur la communique.
    pub region: Option<Region>,
}

/// État de zone DVD d'un lecteur RPC-2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Region {
    /// Zones autorisées (1 à 8) ; vide = aucune zone définie.
    pub mask: u8,
    /// Changements encore possibles par l'utilisateur.
    pub changes_left: u8,
}

impl Region {
    pub fn zones(&self) -> Vec<u8> {
        (0..8).filter(|i| self.mask & (1 << i) == 0).map(|i| i + 1).collect()
    }
}

impl DriveInfo {
    pub fn name(&self) -> String {
        format!("{} {}", self.vendor, self.model).trim().to_string()
    }

    pub fn reads_bluray(&self) -> bool {
        self.reads.contains(&"Blu-ray")
    }
}

/// Recueille les informations du lecteur `dev` (ex. `/dev/sr0`).
pub fn gather(dev: &str) -> Option<DriveInfo> {
    let name = Path::new(dev).file_name()?.to_str()?.to_string();
    let sys = Path::new("/sys/block").join(&name);
    let read = |f: &str| std::fs::read_to_string(sys.join("device").join(f)).map(|s| s.trim().to_string()).unwrap_or_default();
    let mut info = DriveInfo {
        device: dev.to_string(),
        vendor: read("vendor"),
        model: read("model"),
        firmware: read("rev"),
        ..Default::default()
    };
    if info.vendor.is_empty() && info.model.is_empty() {
        return None;
    }
    if let Ok(majmin) = std::fs::read_to_string(sys.join("dev")) {
        if let Ok(udev) = std::fs::read_to_string(format!("/run/udev/data/b{}", majmin.trim())) {
            apply_udev(&mut info, &udev);
        }
    }
    if info.reads.contains(&"DVD") {
        info.region = read_region(dev);
    }
    if let Ok(proc) = std::fs::read_to_string("/proc/sys/dev/cdrom/info") {
        info.max_speed = proc_speed(&proc, &name);
    }
    // Le journal MakeMKV est réécrit à chaque lancement de makemkvcon : on
    // mémorise les détails dès qu'ils ont été lus une fois.
    static CACHE: std::sync::Mutex<Option<std::collections::HashMap<String, (String, Option<String>)>>> = std::sync::Mutex::new(None);
    let key = format!("{}|{}|{}", info.vendor, info.model, info.firmware);
    let found = std::fs::read(crate::makemkv::sdf::debug_log_path())
        .ok()
        .and_then(|log| sdf_details(&String::from_utf8_lossy(&log), &info));
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let cache = cache.get_or_insert_with(Default::default);
    if let Some(d) = found {
        cache.insert(key.clone(), d);
    }
    if let Some((date, serial)) = cache.get(&key).cloned() {
        info.firmware_date = Some(date);
        info.serial = serial;
    }
    Some(info)
}

/// En-tête `sg_io_hdr` de linux/scsi/sg.h (x86_64 / aarch64).
#[repr(C)]
struct SgIoHdr {
    interface_id: libc::c_int,
    dxfer_direction: libc::c_int,
    cmd_len: libc::c_uchar,
    mx_sb_len: libc::c_uchar,
    iovec_count: libc::c_ushort,
    dxfer_len: libc::c_uint,
    dxferp: *mut libc::c_void,
    cmdp: *mut libc::c_uchar,
    sbp: *mut libc::c_uchar,
    timeout: libc::c_uint,
    flags: libc::c_uint,
    pack_id: libc::c_int,
    usr_ptr: *mut libc::c_void,
    status: libc::c_uchar,
    masked_status: libc::c_uchar,
    msg_status: libc::c_uchar,
    sb_len_wr: libc::c_uchar,
    host_status: libc::c_ushort,
    driver_status: libc::c_ushort,
    resid: libc::c_int,
    duration: libc::c_uint,
    info: libc::c_uint,
}

/// Lit l'état de zone DVD (commande SCSI REPORT KEY, format « RPC State »).
/// Lecture seule : ne modifie jamais le lecteur.
fn read_region(dev: &str) -> Option<Region> {
    const SG_IO: libc::c_ulong = 0x2285;
    const SG_DXFER_FROM_DEV: libc::c_int = -3;
    let path = std::ffi::CString::new(dev).ok()?;
    // SAFETY : chemin C valide ; descripteur fermé avant de rendre la main.
    let fd = unsafe { libc::open(path.as_ptr(), libc::O_RDONLY | libc::O_NONBLOCK) };
    if fd < 0 {
        return None;
    }
    let mut cdb: [u8; 12] = [0xA4, 0, 0, 0, 0, 0, 0, 0, 0, 8, 0x08, 0];
    let mut data = [0u8; 8];
    let mut sense = [0u8; 32];
    let mut h = SgIoHdr {
        interface_id: b'S' as libc::c_int,
        dxfer_direction: SG_DXFER_FROM_DEV,
        cmd_len: cdb.len() as u8,
        mx_sb_len: sense.len() as u8,
        iovec_count: 0,
        dxfer_len: data.len() as u32,
        dxferp: data.as_mut_ptr().cast(),
        cmdp: cdb.as_mut_ptr(),
        sbp: sense.as_mut_ptr(),
        timeout: 5000,
        flags: 0,
        pack_id: 0,
        usr_ptr: std::ptr::null_mut(),
        status: 0,
        masked_status: 0,
        msg_status: 0,
        sb_len_wr: 0,
        host_status: 0,
        driver_status: 0,
        resid: 0,
        duration: 0,
        info: 0,
    };
    // SAFETY : structure conforme à sg.h, tampons valides pendant l'appel.
    let r = unsafe { libc::ioctl(fd, SG_IO as _, &mut h as *mut SgIoHdr) };
    unsafe { libc::close(fd) };
    if r < 0 || h.status != 0 || h.host_status != 0 || h.driver_status != 0 {
        return None;
    }
    // Octet 6 : schéma RPC (1 = RPC-2). Sinon, pas de zone matérielle.
    if data[6] != 1 {
        return None;
    }
    Some(Region { mask: data[5], changes_left: data[4] & 0x07 })
}

/// Capacités et média à partir du fichier de base de données udev.
fn apply_udev(info: &mut DriveInfo, udev: &str) {
    let props: BTreeSet<&str> = udev
        .lines()
        .filter_map(|l| l.strip_prefix("E:"))
        .filter(|l| l.ends_with("=1"))
        .map(|l| l.trim_end_matches("=1"))
        .collect();
    let has = |k: &str| props.contains(k);

    for (key, label) in [("ID_CDROM_CD", "CD"), ("ID_CDROM_DVD", "DVD"), ("ID_CDROM_BD", "Blu-ray"), ("ID_CDROM_HDDVD", "HD DVD")] {
        if has(key) {
            info.reads.push(label);
        }
    }
    let writes: [(&[&str], &str); 10] = [
        (&["ID_CDROM_CD_R"], "CD-R"),
        (&["ID_CDROM_CD_RW"], "CD-RW"),
        (&["ID_CDROM_DVD_R"], "DVD-R"),
        (&["ID_CDROM_DVD_RW"], "DVD-RW"),
        (&["ID_CDROM_DVD_PLUS_R"], "DVD+R"),
        (&["ID_CDROM_DVD_PLUS_RW"], "DVD+RW"),
        (&["ID_CDROM_DVD_R_DL", "ID_CDROM_DVD_PLUS_R_DL"], "DVD±R DL"),
        (&["ID_CDROM_DVD_RAM"], "DVD-RAM"),
        (&["ID_CDROM_BD_R"], "BD-R"),
        (&["ID_CDROM_BD_RE"], "BD-RE"),
    ];
    for (keys, label) in writes {
        if keys.iter().any(|k| has(k)) {
            info.writes.push(label);
        }
    }

    for l in udev.lines().filter_map(|l| l.strip_prefix("E:")) {
        if let Some(bus) = l.strip_prefix("ID_BUS=") {
            info.bus = Some(match bus {
                "usb" => "USB".into(),
                "ata" => "SATA".into(),
                "scsi" => "SCSI".into(),
                other => other.to_uppercase(),
            });
        }
    }

    // Média inséré : ID_CDROM_MEDIA_<TYPE>=1 (hors indicateurs d'état).
    if has("ID_CDROM_MEDIA") {
        let media = props
            .iter()
            .filter_map(|k| k.strip_prefix("ID_CDROM_MEDIA_"))
            .filter(|k| !k.starts_with("STATE") && !k.starts_with("SESSION") && !k.starts_with("TRACK"))
            // Le plus spécifique (le plus long) : DVD_PLUS_R_DL plutôt que DVD.
            .max_by_key(|k| k.len())
            .map(media_label);
        info.media = media;
    }
}

fn media_label(k: &str) -> String {
    match k {
        "BD" => "Blu-ray".into(),
        "CD" | "DVD" => k.into(),
        _ => k.replace("_PLUS_", "+").replace("_DL", " DL").replace('_', "-"),
    }
}

/// Vitesse maximale lue dans `/proc/sys/dev/cdrom/info` (une colonne par lecteur).
fn proc_speed(proc: &str, name: &str) -> Option<u32> {
    let row = |label: &str| -> Option<Vec<String>> {
        proc.lines()
            .find(|l| l.starts_with(label))
            .map(|l| l.split_once(':').map(|x| x.1).unwrap_or("").split_whitespace().map(String::from).collect())
    };
    let names = row("drive name")?;
    let col = names.iter().position(|n| n == name)?;
    row("drive speed")?.get(col)?.parse().ok()
}

/// Date du firmware et numéro de série, à partir de l'identifiant SDF de MakeMKV
/// `VENDEUR_MODELE_REV_AAAAMMJJhhmmss_SERIE`.
fn sdf_details(log: &str, info: &DriveInfo) -> Option<(String, Option<String>)> {
    let norm = |s: &str| s.split_whitespace().collect::<Vec<_>>().join("_");
    let prefix = format!("{}_{}_{}_", norm(&info.vendor), norm(&info.model), norm(&info.firmware));
    // Toute mention de l'identifiant convient (« SDF auto », « SDF disabled »…).
    let id = log.split_whitespace().find(|w| w.starts_with(&prefix))?;
    let rest = &id[prefix.len()..];
    let (stamp, serial) = match rest.split_once('_') {
        Some((d, s)) => (d, Some(s.to_string()).filter(|s| !s.is_empty())),
        None => (rest, None),
    };
    if stamp.len() < 8 || !stamp[..8].chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some((format!("{}-{}-{}", &stamp[..4], &stamp[4..6], &stamp[6..8]), serial))
}

#[cfg(test)]
mod tests {
    use super::*;

    const UDEV: &str = "E:ID_CDROM=1\nE:ID_CDROM_CD=1\nE:ID_CDROM_CD_R=1\nE:ID_CDROM_DVD=1\nE:ID_CDROM_DVD_R=1\n\
        E:ID_CDROM_DVD_PLUS_R_DL=1\nE:ID_CDROM_DVD_RAM=1\nE:ID_BUS=usb\nE:ID_CDROM_MEDIA=1\nE:ID_CDROM_MEDIA_DVD=1\n\
        E:ID_CDROM_MEDIA_DVD_PLUS_R_DL=1\nE:ID_CDROM_MEDIA_STATE=complete\nE:ID_CDROM_MEDIA_TRACK_COUNT=1\n";

    #[test]
    fn parses_udev() {
        let mut i = DriveInfo::default();
        apply_udev(&mut i, UDEV);
        assert_eq!(i.reads, vec!["CD", "DVD"]);
        assert_eq!(i.writes, vec!["CD-R", "DVD-R", "DVD±R DL", "DVD-RAM"]);
        assert_eq!(i.bus.as_deref(), Some("USB"));
        assert_eq!(i.media.as_deref(), Some("DVD+R DL"));
        assert!(!i.reads_bluray());
        assert_eq!(media_label("BD_RE"), "BD-RE");
        assert_eq!(media_label("BD"), "Blu-ray");
    }

    #[test]
    fn parses_speed() {
        let p = "drive name:\t\tsr1\tsr0\ndrive speed:\t\t48\t24\n";
        assert_eq!(proc_speed(p, "sr0"), Some(24));
        assert_eq!(proc_speed(p, "sr1"), Some(48));
    }

    #[test]
    fn parses_sdf_details() {
        let info = DriveInfo { vendor: "HL-DT-ST".into(), model: "DVDRAM GSA-T50N".into(), firmware: "RE06".into(), ..Default::default() };
        let log = "SDF disabled: HL-DT-ST_DVDRAM_GSA-T50N_RE06_20090730123456_M000EXEMPLE\n";
        assert_eq!(sdf_details(log, &info), Some(("2009-07-30".into(), Some("M000EXEMPLE".into()))));
    }
}
