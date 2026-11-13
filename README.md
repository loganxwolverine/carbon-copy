# Carbon Copy

<img src="data/icons/hicolor/scalable/apps/io.github.carboncopy.CarbonCopy.svg" width="96" alt="">

*Une copie conforme de vos disques.*

Interface GTK4 / libadwaita pour **MakeMKV** sous Fedora (GNOME), qui automatise
le rip d'un DVD, d'un Blu-ray ou d'un Blu-ray UHD.

## Ce que fait l'application

| Étape | Fonctionnement |
|---|---|
| **MakeMKV** | Utilise `makemkvcon` natif s'il est à jour, sinon installe ou met à jour le Flatpak Flathub `com.makemkv.MakeMKV` (mode `--user`, sans sudo). |
| **Clé bêta** | Récupère la clé publique sur le forum officiel au plus une fois par jour, et immédiatement si MakeMKV la déclare expirée. Elle est écrite dans `~/.MakeMKV/settings.conf` (avec une sauvegarde `.bak`) et un toast vous prévient. Une clé achetée (qui ne commence pas par `T-`) n'est jamais modifiée. |
| **Blocage SDF** | MakeMKV 2.x peut se figer à « Balayage des lecteurs CD-ROM » avec certains lecteurs. Si aucun lecteur n'est listé en 60 s, l'application tue le processus, lit l'identifiant du lecteur dans `~/MakeMKV_log.txt` (makemkvcon est lancé avec `--debug`), écrit `sdf_Stop = "<identifiant>"` dans `~/.MakeMKV/settings.conf` et relance la commande. |
| **Disque** | Détecte l'insertion via l'ioctl `CDROM_DRIVE_STATUS`, puis lance `makemkvcon -r info`. Affiche le type (DVD, Blu-ray ou UHD), le nom et la durée. |
| **Pistes** | Le titre principal (le plus long) est choisi d'office. Sont pré-cochées : l'audio dans votre langue, la VO, et **uniquement les sous-titres forcés** de votre langue. Les sous-titres complets ne sont cochés que si aucune piste audio n'existe dans votre langue. |
| **Nom** | Le nom du disque est nettoyé, puis complété via TMDB si une clé est configurée. Exemple : `Le_Grand_Film_2019.mkv`. |
| **Rip** | MakeMKV extrait **toutes** les pistes (règle `+sel:all`, appliquée temporairement puis restaurée). Ensuite `mkvmerge` ne garde que les pistes cochées et pose les drapeaux `default` et `forced`. |

## Prérequis (Fedora 40 ou plus récent)

```bash
sudo dnf install gcc rust cargo gtk4-devel libadwaita-devel flatpak
```

`mkvmerge` est conseillé en natif. S'il est absent, l'application installe le Flatpak MKVToolNix.

```bash
sudo dnf install mkvtoolnix
```

Les lecteurs Blu-ray ont besoin du module `sg` (périphériques `/dev/sg*`) :

```bash
echo sg | sudo tee /etc/modules-load.d/sg.conf
```

```bash
sudo modprobe sg
```

Votre utilisateur doit pouvoir lire `/dev/sr0`. Le groupe `cdrom` suffit, et c'est le cas par défaut sous GNOME.

## Compilation et installation

```bash
cargo build --release
```

```bash
install -Dm755 target/release/carbon-copy ~/.local/bin/carbon-copy
```

```bash
install -Dm644 data/io.github.carboncopy.CarbonCopy.desktop ~/.local/share/applications/io.github.carboncopy.CarbonCopy.desktop
```

```bash
install -Dm644 data/icons/hicolor/scalable/apps/io.github.carboncopy.CarbonCopy.svg ~/.local/share/icons/hicolor/scalable/apps/io.github.carboncopy.CarbonCopy.svg
```

## Tests

```bash
cargo test
```

Le test d'intégration ci-dessous a besoin de `mkvmerge` :

```bash
cargo test -- --ignored
```

Pour tester l'interface sans lecteur physique, placez un faux `makemkvcon` en tête du `PATH`, puis :

```bash
CARBON_COPY_FAKE_DISC=/dev/sr0 cargo run
```

## Configuration

Les préférences sont dans `~/.config/carbon-copy/config.toml` (l'ancien dossier `~/.config/mkv-ripper` est repris automatiquement). On peut aussi les modifier via Menu → Préférences :

- langue préférée ;
- conservation de la VO ;
- clé TMDB (v3 ou jeton v4) ;
- durée minimale des titres ;
- éjection du disque à la fin ;
- source de la clé bêta.

## Organisation du code

```
src/
├── main.rs            point d'entrée (adw::Application)
├── backend.rs         orchestration tokio : démarrage, analyse, rip
├── events.rs          événements tâches de fond → interface (async_channel)
├── process.rs         exécution asynchrone, lecture ligne à ligne, Flatpak
├── drive.rs           détection d'insertion (ioctl), éjection
├── beta_key.rs        récupération et installation de la clé bêta
├── selection.rs       règles de pré-sélection (sous-titres forcés…)
├── metadata.rs        nettoyage du nom de disque, TMDB, nom de fichier
├── remux.rs           mkvmerge : identification, correspondance, remux
├── config.rs          préférences TOML
├── makemkv/
│   ├── robot.rs       parseur du mode robot (MSG/PRGV/DRV/CINFO/TINFO/SINFO)
│   ├── disc.rs        modèle disque / titres / flux
│   ├── cli.rs         sonde, analyse, extraction
│   ├── install.rs     vérification, installation et mise à jour
│   └── settings.rs    lecture et écriture de ~/.MakeMKV/settings.conf
└── ui/
    ├── mod.rs         contrôleur (état + gestion des événements)
    ├── widgets.rs     construction des widgets
    └── prefs.rs       fenêtre de préférences
```
