//! Lancement automatique à l'ouverture de session (XDG Autostart), pour un
//! HTPC piloté par la télécommande web.

use std::path::PathBuf;

fn path() -> PathBuf {
    gtk::glib::user_config_dir().join("autostart").join(format!("{}.desktop", crate::APP_ID))
}

pub fn is_enabled() -> bool {
    path().exists()
}

pub fn set_enabled(enabled: bool) -> anyhow::Result<()> {
    let p = path();
    if !enabled {
        if p.exists() {
            std::fs::remove_file(&p)?;
        }
        return Ok(());
    }
    // Chemin absolu de l'exécutable courant (paquet système ou ~/.local/bin).
    let exe = std::env::current_exe().ok().filter(|e| e.exists()).map(|e| e.display().to_string());
    let exec = exe.map(|e| format!("\"{}\"", e.replace('"', "\\\""))).unwrap_or_else(|| "intact-bitstream".into());
    let entry = format!(
        "[Desktop Entry]\nType=Application\nName={name}\nComment=Démarré à l'ouverture de session\nExec={exec}\nIcon={id}\nTerminal=false\nX-GNOME-Autostart-enabled=true\n",
        name = crate::ui::APP_NAME,
        id = crate::APP_ID,
    );
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&p, entry)?;
    Ok(())
}
