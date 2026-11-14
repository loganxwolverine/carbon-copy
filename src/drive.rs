//! Détection du lecteur optique et de l'insertion d'un disque.
//!
//! Interroger `makemkvcon` en boucle serait lent (plusieurs secondes) et
//! réveillerait le lecteur. On utilise donc l'ioctl Linux `CDROM_DRIVE_STATUS`
//! sur `/dev/srN`, quasi instantané, puis on ne lance l'analyse MakeMKV que
//! lorsqu'un disque vient d'être inséré.

use std::collections::HashMap;
use std::ffi::CString;
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use crate::events::{emit, AppEvent, Tx};

const CDROM_DRIVE_STATUS: libc::c_ulong = 0x5326;
const CDROMEJECT: libc::c_ulong = 0x5309;
const CDSL_CURRENT: libc::c_int = libc::c_int::MAX;
const CDS_DISC_OK: libc::c_int = 4;

/// Lecteurs optiques présents (`/dev/sr0`, `/dev/sr1`…).
pub fn optical_drives() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir("/dev")
        .map(|d| {
            d.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| n.starts_with("sr") && n[2..].chars().all(|c| c.is_ascii_digit()))
                .map(|n| format!("/dev/{n}"))
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

/// Ouvre le périphérique en mode non bloquant (indispensable : sans
/// `O_NONBLOCK`, `open` échoue tant qu'il n'y a pas de disque).
fn with_device<T>(dev: &str, f: impl FnOnce(libc::c_int) -> T) -> Option<T> {
    let c = CString::new(dev).ok()?;
    // SAFETY : chemin C valide, descripteur fermé juste après usage.
    let fd = unsafe { libc::open(c.as_ptr(), libc::O_RDONLY | libc::O_NONBLOCK) };
    if fd < 0 {
        return None;
    }
    let r = f(fd);
    unsafe { libc::close(fd) };
    Some(r)
}

/// Un disque lisible est-il présent ?
pub fn has_disc(dev: &str) -> bool {
    // SAFETY : ioctl documenté de linux/cdrom.h, argument entier.
    with_device(dev, |fd| unsafe { libc::ioctl(fd, CDROM_DRIVE_STATUS as _, CDSL_CURRENT) } == CDS_DISC_OK)
        .unwrap_or(false)
}

const CDROM_LOCKDOOR: libc::c_ulong = 0x5329;

/// Éjecte le disque.
///
/// MakeMKV verrouille le tiroir pendant la lecture, et le lecteur peut rester
/// occupé quelques secondes après la fin d'un rip. On déverrouille donc le
/// tiroir, on réessaie plusieurs fois, puis on se rabat sur la commande
/// `eject` (util-linux), qui démonte aussi le disque s'il a été monté.
/// Fonction bloquante : à appeler via `spawn_blocking`.
pub fn eject(dev: &str) -> Result<(), String> {
    let mut last_err = String::from("périphérique inaccessible");
    for attempt in 0..5 {
        if attempt > 0 {
            std::thread::sleep(Duration::from_secs(1));
        }
        let r = with_device(dev, |fd| {
            // SAFETY : ioctls documentés de linux/cdrom.h, argument entier.
            unsafe { libc::ioctl(fd, CDROM_LOCKDOOR as _, 0) };
            if unsafe { libc::ioctl(fd, CDROMEJECT as _, 0) } == 0 {
                Ok(())
            } else {
                Err(std::io::Error::last_os_error().to_string())
            }
        });
        match r {
            Some(Ok(())) => return Ok(()),
            Some(Err(e)) => last_err = e,
            None => last_err = std::io::Error::last_os_error().to_string(),
        }
    }
    match std::process::Command::new("eject").arg(dev).output() {
        Ok(o) if o.status.success() => Ok(()),
        Ok(o) => Err(format!("{last_err} ; eject : {}", String::from_utf8_lossy(&o.stderr).trim())),
        Err(_) => Err(last_err),
    }
}

/// Boucle de surveillance : émet `DriveState` à chaque changement d'état.
pub async fn watch(tx: Tx, cancel: CancellationToken) {
    let mut known: HashMap<String, bool> = HashMap::new();
    // Aide au développement : simule un disque inséré sans lecteur physique
    // (ex. CARBON_COPY_FAKE_DISC=/dev/sr0 avec un faux makemkvcon dans le PATH).
    if let Ok(dev) = std::env::var("CARBON_COPY_FAKE_DISC") {
        emit(&tx, AppEvent::DriveState { device: dev, has_disc: true });
        cancel.cancelled().await;
        return;
    }
    loop {
        for dev in optical_drives() {
            // L'ioctl est rapide mais reste un appel système bloquant :
            // on le sort du runtime asynchrone par précaution.
            let d = dev.clone();
            let present = tokio::task::spawn_blocking(move || has_disc(&d)).await.unwrap_or(false);
            if known.get(&dev) != Some(&present) {
                known.insert(dev.clone(), present);
                emit(&tx, AppEvent::DriveState { device: dev, has_disc: present });
            }
        }
        tokio::select! {
            _ = cancel.cancelled() => return,
            _ = tokio::time::sleep(Duration::from_secs(2)) => {}
        }
    }
}
