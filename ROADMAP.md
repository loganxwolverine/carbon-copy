# Feuille de route — Intact Bitstream

Propositions d'amélioration classées par pertinence (gain pour l'utilisateur
rapporté à l'effort). Elles sont issues des rips réels : DVD *Ice Storm*,
Blu-ray *L'Agence tous risques*, UHD 4K *Wolf Creek* (14/11/2026).

Légende effort : 🟢 petit · 🟡 moyen · 🔴 important — ✅ fait, ⏳ à valider
sur disque.

---

## ✅ Fait — 14/11/2026

### Sous-titres forcés jamais reconnus après extraction (bug) ✅
**Constat (UHD *Wolf Creek*)** : MakeMKV crée pour chaque piste de
sous-titres une piste « forcés seuls » virtuelle ; sur ce disque, elles étaient
vides et n'ont pas été écrites. La vraie piste forcée (50 sous-titres) n'est
pas marquée comme telle, et MakeMKV n'écrit jamais le drapeau `forced` dans
le MKV. Résultat : ré-extraction complète inutile (+44 min), remux de 68 min
**sans sous-titres**, rip de 2 h 40 au lieu de ~50 min.
**Corrigé** :
- une piste de sous-titres est reconnue comme forcée à son faible nombre de
  sous-titres face aux autres pistes de la même langue (test de non-régression
  reproduisant le disque) ;
- si seuls des sous-titres manquent, plus de ré-extraction (MakeMKV n'écrit
  pas les pistes vides : relancer ne servirait à rien) — le film est finalisé
  sans elles, avec un message clair ;
- le journal liste les pistes trouvées et celles qui manquent.
**Validé** sur *Freddy : Les Griffes de la nuit* (Blu-ray, 14/11) : pistes
forcées vides détectées, aucune ré-extraction, aucun remux (mkvpropedit).

### Désigner la piste HD sans remux ✅ ⏳
**Constat** : anglais en DTS-HD MA + cœur DTS + AC-3 (Blu-ray) ; la règle ne
savait isoler que le cœur, d'où un remux de 26 min (Blu-ray) et 68 min (UHD).
**Corrigé** : la règle utilise aussi les critères MakeMKV `lossless` et
`lossy` (ex. `+sel:(audio&eng&lossless)`). **Validé** le 17/11 sur *Priest*.
Note d'origine : **À valider** sur le prochain
Blu-ray : le journal doit afficher « Aucun remux nécessaire ». En cas de
mauvaise interprétation par MakeMKV, la vérification le détecte (repli
existant).

### Lecteur qui ne répond plus ✅
**Constat** : après l'analyse, le lecteur a refusé toute lecture ; MakeMKV
répondait « Échec de l'ouverture du disque » sans explication.
**Corrigé** : 3 tentatives espacées de 4 s (cas d'un lecteur encore occupé),
puis message « Le lecteur ne répond plus : éjectez puis réinsérez le disque ».

### Relance propre après un changement de sélection ✅
Validé : deux relances successives, disque rouvert du premier coup.
L'annulation attend désormais la fin réelle du processus MakeMKV avant d'en
lancer un autre ; sur Blu-ray le délai avant relance passe de 1,5 s à 5 s
(chaque relance refait le traitement AACS / BD+).

---

## P1 — À faire en priorité

### 1. Publier la version 0.1.0 ✅
Publiée le 15/11/2026 sous le nouveau nom **Intact Bitstream** : paquets
Ubuntu, Debian, Fedora et openSUSE, testés en conteneur (installation,
menu, lancement, reprise des réglages de « Carbon Copy »).

### 2. Valider en conditions réelles 🟢
- ✅ critère `lossless` — validé le 17/11 sur *Priest* (Blu-ray) : règle
  `+sel:(audio&eng&lossless),+sel:(audio&fra),-sel:(audio&fra&core)`,
  pistes conformes, « Aucun remux nécessaire » (mkvpropedit seul) ; fichier
  final : DTS-HD MA anglais + DTS-HD MA français + sous-titres forcés ;
- MakeMKV 2.0 donne les langues en `fra` (et non plus `fre`) : la
  comparaison le gère déjà, à confirmer sur un rip réussi ;
- temps restant et vitesse affichés (mesuré : 17–27 Mo/s, limité par
  l'écriture sur le NAS) ;
- progression sur l'icône de la barre des tâches KDE ;
- choix du titre principal par la durée TMDB (cinéma / version longue).

### 3. Proposer la vraie piste forcée dès l'analyse 🟡
Sur *Wolf Creek*, la piste « forcés » proposée par MakeMKV était vide alors
qu'une piste séparée contenait les sous-titres forcés. On ne peut pas le
savoir avant extraction ; piste possible : extraire aussi les petites pistes
de la langue choisie (quelques Mo) et garder la bonne après coup.

---

## P2 — Utile

### 4. Paquet Arch 🟢
Retiré de la v0.1.0. La CI échouait à l'édition des liens d'`aws-lc-sys`
(LTO de makepkg) ; corrigé par `options=('!lto')` dans le PKGBUILD mais
non testé. À réactiver dans la CI puis valider sur une vraie machine Arch.

### 5. Étiquette HDR / Dolby Vision dans le nom des UHD 🟢
« 2160p HEVC » ne dit pas si le film est en HDR10 ou Dolby Vision.

### 6. Afficher les erreurs de lecture du disque 🟢
Résumer sur l'écran final les erreurs signalées par MakeMKV (*Read Error*,
*HashCheck Error*), en distinguant les fichiers leurres de la protection des
vrais défauts du film.

---

## P3 — Confort et finition

### 7. Remux plus rapide quand il reste nécessaire 🔴
Le remux lit puis réécrit tout sur le NAS (≈ 13 Mo/s mesurés). Pistes :
écrire la copie brute sur un disque local quand l'espace le permet.

### 8. Avertissements GTK de mise en page 🟢
« Trying to measure GtkBox … for height of 597, but it needs at least 671 »
au démarrage ; sans effet visible.

### 9. Nom de fichier : codec de la meilleure piste ? 🟢
Le nom décrit la piste audio dans la langue préférée (convention Plex /
Radarr). À rediscuter si l'on préfère la meilleure piste du fichier.

---

## Nouvelles fonctionnalités (demandées le 17/11/2026)

### 10. Ripper des séries TV (DVD et Blu-ray) 🔴
**Besoin** : une saison tient sur plusieurs disques ; il faut savoir de
quelle série et de quelle saison il s'agit, quels épisodes contient chaque
disque, et les ranger comme Plex / Jellyfin / Kodi l'attendent :

```
Séries/
└── Nom de la série (2008) [tmdbid-1396]/
    └── Season 01/
        ├── Nom de la série (2008) - S01E01 - Titre de l'épisode.mkv
        └── Nom de la série (2008) - S01E02 - Titre de l'épisode.mkv
```

**Pistes à étudier** :
- *Identifier la série* : étiquette du disque (`BREAKING_BAD_S1_D2`),
  recherche TMDB (API TV), choix confirmé par l'utilisateur ;
- *Identifier la saison et le disque* : étiquette, puis demande à
  l'utilisateur (« saison 1, disque 2 ») ; mémoriser la session de rip pour
  enchaîner les disques de la saison sans tout ressaisir ;
- *Reconnaître les épisodes* : titres de durée proche de celle des
  épisodes TMDB ; écarter les titres « lecture intégrale » (somme des
  épisodes) et les doublons ; garder l'ordre des playlists / titres du
  disque ;
- *Numéroter* : continuer la numérotation d'un disque à l'autre (le
  disque 2 commence après le dernier épisode du disque 1), avec correction
  possible avant le rip ;
- *Ripper plusieurs titres d'un coup* : aujourd'hui l'application extrait un
  seul titre (le film) ; il faut une file d'épisodes, avec progression
  globale et par épisode ;
- *Bonus* : les ignorer, ou les ranger dans `Season 00` / `Extras`.

### 11. Télécommande web 🔴
**Besoin** : le HTPC est allumé avec l'application lancée par défaut,
mais l'écran est éteint. Depuis un téléphone ou un autre PC du réseau
local : voir le disque inséré, confirmer le film (ou la série), lancer le
rip et suivre son avancement.

**Pistes à étudier** :
- serveur web intégré à l'application, activable dans les préférences
  (port configurable), accessible seulement sur le réseau local ;
- protection par code ou jeton (affiché dans les préférences / QR code) ;
- pages : état du lecteur, résultat de l'analyse et fiche TMDB, choix des
  pistes (réglages par défaut), bouton Ripper / Annuler, progression en
  direct (vitesse, temps restant), vidéothèque des derniers rips ;
- avancement poussé en temps réel (Server-Sent Events ou WebSocket) à
  partir des événements déjà émis pour l'interface GTK ;
- lancement automatique de l'application à l'ouverture de session.
