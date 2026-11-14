# Feuille de route — Carbon Copy

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

### Désigner la piste HD sans remux ✅ ⏳
**Constat** : anglais en DTS-HD MA + cœur DTS + AC-3 (Blu-ray) ; la règle ne
savait isoler que le cœur, d'où un remux de 26 min (Blu-ray) et 68 min (UHD).
**Corrigé** : la règle utilise aussi les critères MakeMKV `lossless` et
`lossy` (ex. `+sel:(audio&eng&lossless)`). **À valider** sur le prochain
Blu-ray : le journal doit afficher « Aucun remux nécessaire ». En cas de
mauvaise interprétation par MakeMKV, la vérification le détecte (repli
existant).

### Lecteur qui ne répond plus ✅
**Constat** : après l'analyse, le lecteur a refusé toute lecture ; MakeMKV
répondait « Échec de l'ouverture du disque » sans explication.
**Corrigé** : 3 tentatives espacées de 4 s (cas d'un lecteur encore occupé),
puis message « Le lecteur ne répond plus : éjectez puis réinsérez le disque ».

### Relance propre après un changement de sélection ✅
L'annulation attend désormais la fin réelle du processus MakeMKV avant d'en
lancer un autre ; sur Blu-ray le délai avant relance passe de 1,5 s à 5 s
(chaque relance refait le traitement AACS / BD+).

---

## P1 — À faire en priorité

### 1. Publier la version 0.1.0 🟢
Commits locaux prêts. Bloqué par le jeton GitHub : lancer
`gh auth refresh -h github.com -s workflow`, puis pousser et créer le tag
`v0.1.0` pour que la CI construise les paquets.

### 2. Valider en conditions réelles 🟢
- critère `lossless` (voir plus haut) ;
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

### 4. Tester le paquet Arch 🟢
Test local interrompu faute de mémoire ; à vérifier via la CI.

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
