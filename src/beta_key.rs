//! Gestion autonome de la clé bêta publique de MakeMKV.
//!
//! Pendant la bêta, l'auteur de MakeMKV publie une clé gratuite sur le forum
//! officiel ; elle est renouvelée régulièrement et l'ancienne expire. Ce module :
//! 1. récupère la clé courante (requête HTTP légère, une fois par jour max) ;
//! 2. l'écrit dans `~/.MakeMKV/settings.conf` si elle diffère ;
//! 3. ne touche JAMAIS à une clé achetée (format différent de `T-…`).
//!
//! La détection « clé expirée » est faite par `cli::probe` (messages MakeMKV),
//! qui déclenche alors une récupération forcée.

use anyhow::{bail, Context, Result};
use regex::Regex;

use crate::makemkv::settings;

/// Messages MakeMKV signalant une clé/évaluation expirée ou une version trop ancienne.
pub fn is_key_problem(code: u32, text: &str) -> bool {
    // 5021 : « This application version is too old… enter a registration key »
    // 5095 : « Evaluation period has expired… »
    // 5055 / 5056 : clé invalide / expirée (selon versions).
    const CODES: [u32; 4] = [5021, 5055, 5056, 5095];
    let t = text.to_lowercase();
    CODES.contains(&code)
        || t.contains("evaluation period has expired")
        || t.contains("registration key")
        || t.contains("too old")
}

/// Extrait une clé bêta d'une page (HTML du forum ou texte brut d'un miroir).
pub fn extract_key(body: &str) -> Option<String> {
    // Sur le forum, la clé est dans un bloc <code>…</code> ; on cherche d'abord
    // là pour éviter de capturer une vieille clé citée ailleurs dans le fil.
    let code_block = Regex::new(r"(?s)<code>\s*(T-[A-Za-z0-9@_]{30,})\s*</code>").ok()?;
    if let Some(c) = code_block.captures(body) {
        return Some(c[1].to_string());
    }
    let any = Regex::new(r"\bT-[A-Za-z0-9@_]{30,}").ok()?;
    any.find(body).map(|m| m.as_str().to_string())
}

/// Télécharge la clé courante depuis la première source qui répond.
pub async fn fetch_key(http: &reqwest::Client, sources: &[String]) -> Result<String> {
    let mut last_err = None;
    for url in sources {
        let res = async {
            let body = http
                .get(url)
                .send()
                .await?
                .error_for_status()?
                .text()
                .await
                .context("lecture de la page")?;
            extract_key(&body).with_context(|| format!("aucune clé trouvée sur {url}"))
        }
        .await;
        match res {
            Ok(k) => return Ok(k),
            Err(e) => last_err = Some(e),
        }
    }
    match last_err {
        Some(e) => Err(e),
        None => bail!("aucune source de clé configurée"),
    }
}

/// Résultat d'une tentative de mise à jour.
#[derive(Debug, PartialEq, Eq)]
pub enum KeyUpdate {
    /// Une clé achetée est en place : on n'y touche pas.
    PurchasedKey,
    /// La clé en place est déjà la plus récente.
    UpToDate,
    /// Nouvelle clé écrite.
    Updated(String),
}

/// Récupère la clé en ligne et l'installe si besoin.
pub async fn update_key(http: &reqwest::Client, sources: &[String]) -> Result<KeyUpdate> {
    let current = settings::current_key();
    if let Some(k) = &current {
        if !settings::is_beta_key(k) {
            return Ok(KeyUpdate::PurchasedKey);
        }
    }
    let fresh = fetch_key(http, sources).await?;
    if current.as_deref() == Some(fresh.as_str()) {
        return Ok(KeyUpdate::UpToDate);
    }
    settings::set(settings::KEY_ENTRY, Some(&fresh)).context("écriture de settings.conf")?;
    Ok(KeyUpdate::Updated(fresh))
}

/// Masque une clé pour l'affichage dans le journal.
pub fn mask(key: &str) -> String {
    let n = key.chars().count();
    if n <= 10 {
        return "…".into();
    }
    format!("{}…{}", key.chars().take(6).collect::<String>(), key.chars().skip(n - 4).collect::<String>())
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "T-AbCdEfGhIjKlMnOpQrStUvWxYz0123456789@_AbCdEfGhIjKlMnOpQrStUvWx";

    #[test]
    fn extracts_from_forum_html() {
        let html = format!(
            "<p>Ancienne : T-OLDOLDOLDOLDOLDOLDOLDOLDOLDOLDOLDOLD</p>\
             <div class=\"codebox\"><pre><code>{KEY}</code></pre></div>"
        );
        assert_eq!(extract_key(&html).as_deref(), Some(KEY));
    }

    #[test]
    fn extracts_from_raw_text() {
        assert_eq!(extract_key(&format!("{KEY}\n")).as_deref(), Some(KEY));
        assert_eq!(extract_key("rien ici"), None);
    }

    #[test]
    fn detects_expiry_messages() {
        assert!(is_key_problem(5021, "This application version is too old."));
        assert!(is_key_problem(0, "Evaluation period has expired"));
        assert!(!is_key_problem(1005, "MakeMKV v1.17.8 linux(x64-release) started"));
    }
}
