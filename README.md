# Galette

Ripper de CD audio **sécurisé**, **libre** (GPL-3.0) et **multiplateforme** (Linux, Windows, macOS).

L'objectif : la fiabilité d'EAC et la simplicité de dBpoweramp CD Ripper.
On insère un CD, Galette le reconnaît sur MusicBrainz, extrait l'audio sans erreur,
vérifie le résultat avec AccurateRip et produit des fichiers correctement tagués
(FLAC, ALAC, MP3, AIFF ou WAV, et AAC sur macOS).

> État : début de développement. Sous Linux, macOS et Windows, Galette reconnaît le CD sur MusicBrainz,
> l'extrait avec correction du décalage du lecteur, vérifie chaque piste avec
> AccurateRip et produit des FLAC tagués avec leur pochette, un fichier `.cue`
> (pré-gaps, ISRC, MCN) et un log. Mode sécurisé avec relectures et pointeurs C2.

## Installer (Linux)

Construire les paquets (il faut Rust, les dépendances ci-dessous et `cargo install tauri-cli`) :

```sh
cd crates/galette-gui && cargo tauri build
```

- Debian / Ubuntu : `sudo apt install ./target/release/bundle/deb/Galette_0.3.0_amd64.deb`
- Toute distribution : `target/release/bundle/appimage/Galette_0.3.0_amd64.AppImage`, à rendre exécutable puis lancer

Galette apparaît alors dans le menu Multimédia. Pour accéder au lecteur, l'utilisateur doit
faire partie du groupe `cdrom` (c'est le cas par défaut sous Debian et Ubuntu).

## Installer (macOS)

Ouvrir `Galette_0.3.0_aarch64.dmg` et glisser Galette dans Applications.
L'application n'est pas signée par un certificat Apple : au premier lancement,
macOS refuse de l'ouvrir. Aller alors dans Réglages Système → Confidentialité et
sécurité, puis « Ouvrir quand même ».

Pour la construire (il faut Rust, les outils en ligne de commande de Xcode et `cargo install tauri-cli`) :

```sh
export CARGO_TARGET_DIR=~/.cache/galette-target   # hors de ~/Documents, voir ci-dessous
cd crates/galette-gui && cargo tauri build
```

Le `.app` et le `.dmg` sont dans `$CARGO_TARGET_DIR/release/bundle/`.

Galette démonte le CD pendant qu'il le lit (macOS l'exige pour accéder directement
au lecteur), puis le rend au système. Si l'app Musique s'ouvre à l'insertion d'un CD,
fermez-la : elle empêche Galette d'accéder au disque. Un programme lancé depuis
le dossier Documents ne peut pas ouvrir le lecteur (protection de la vie privée
de macOS) : d'où le dossier de compilation ailleurs.

## Installer (Windows)

Lancer `Galette_0.4.0_x64-setup.exe` (Windows 10 ou 11, 64 bits). Les droits
administrateur ne sont pas nécessaires pour ripper.
L'installateur n'est pas signé par un certificat : Windows SmartScreen affiche
« Windows a protégé votre ordinateur ». Cliquer sur « Informations complémentaires »,
puis « Exécuter quand même ». Si Smart App Control est activé, il bloque
l'installateur sans proposer de passer outre.

Pour le construire, il faut Rust (toolchain MSVC), les Visual Studio Build Tools
avec « Développement Desktop en C++ » et `cargo install tauri-cli`. Smart App
Control bloque les programmes intermédiaires de la compilation : le désactiver
(Sécurité Windows → Contrôle des applications et du navigateur).

```sh
cd crates/galette-gui && cargo tauri build
```

L'installateur est dans `target/release/bundle/nsis/`.

## Essayer

Il faut [Rust](https://rustup.rs). Pour l'application graphique sous Linux (Debian/Ubuntu) :

```sh
sudo apt install libwebkit2gtk-4.1-dev build-essential libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev pkg-config
cargo run --release -p galette-gui
```

En ligne de commande :

```sh
cargo run -p galette-cli -- info                    # CD dans /dev/sr0 (Linux), premier lecteur (macOS, Windows)
cargo run -p galette-cli -- info --device /dev/sr1  # macOS : --device disk4, Windows : --device E:
cargo run --release -p galette-cli -- rip -o ~/Musique             # FLAC + tags + pochette
cargo run --release -p galette-cli -- rip -o ~/Musique --format wav
cargo run --release -p galette-cli -- rip -o ~/Musique --format mp3 --mp3-quality v0   # ou v2, 320
cargo run --release -p galette-cli -- rip -o ~/Musique --format aiff
cargo run --release -p galette-cli -- rip -o ~/Musique --format alac   # Apple Lossless (.m4a)
cargo run --release -p galette-cli -- rip -o ~/Musique --format aac    # macOS : AAC 256 kbit/s (.m4a)
cargo run --release -p galette-cli -- rip -o ~/Musique --mode burst  # une seule lecture
cargo run --release -p galette-cli -- rip --folder "{albumartist}/{year} - {album}" --name "{track} {title}"
cargo run --release -p galette-cli -- offset   # trouve le décalage du lecteur (disque connu d'AccurateRip)
cargo run --release -p galette-cli -- cache    # vérifie que les relectures lisent vraiment le disque
# Sans lecteur, à partir d'une TOC au format MusicBrainz :
cargo run -p galette-cli -- info --toc "1 12 242387 150 21636 50551 70783 90795 113395 133171 142126 159578 181123 198419 218045"
cargo test
```

## Mode sécurisé

Par défaut, chaque piste est lue une fois en demandant au lecteur ses pointeurs C2
(les octets qu'il n'a pas pu corriger). Si cette lecture est sans erreur C2 et
correspond à AccurateRip, la piste est prouvée correcte. Sinon elle est relue
entièrement après avoir vidé le cache du lecteur, et les deux lectures sont
comparées. Chaque secteur différent ou signalé par C2 est relu (jusqu'à 20 fois,
cache vidé à chaque fois) jusqu'à obtenir 3 lectures identiques sans erreur C2,
ou jusqu'à ce que la piste corresponde à AccurateRip. Un secteur qui n'a jamais
été lu sans erreur après 5 tours est abandonné : sur un disque rayé, chaque
lecture d'un secteur abîmé peut prendre plusieurs secondes. Les secteurs qui
n'y parviennent pas sont notés comme suspects dans le log.

Comparer deux lectures ne prouve rien si la seconde sort du cache du lecteur :
avant chaque relecture, Galette lit donc ailleurs sur le disque de quoi le
vider (2 Mio au moins). Le bouton « Tester le cache du lecteur » des réglages
(ou `galette cache`) mesure le cache au chronomètre (une relecture servie par
le cache est des dizaines de fois plus rapide) et cherche combien il faut lire
pour le vider ; le résultat est retenu pour les rips suivants.

## Décalage du lecteur

Chaque modèle de lecteur lit l'audio décalé de quelques échantillons. Galette
prend ce décalage dans la base d'AccurateRip ; pour un lecteur qui n'y est pas,
« Détecter avec ce disque » (ou `galette offset`) lit deux pistes d'un disque
connu d'AccurateRip et cherche le décalage, à ±3000 échantillons près, auquel
elles correspondent à la base. Un disque pressé plusieurs fois peut
correspondre à plusieurs décalages (les pressages sont souvent décalés entre
eux) : il faut alors un second disque. Galette garde les décalages communs aux
deux, avec leur plus faible confirmation, et retient celui qui est nettement
mieux confirmé que les autres sur chaque disque (sinon, il demande un
troisième disque).

## ReplayGain

Galette mesure le volume perçu de chaque piste (norme EBU R128 / ITU BS.1770)
et ajoute les tags ReplayGain 2.0 (`REPLAYGAIN_TRACK_GAIN`, `…_PEAK`, et ceux
de l'album pour un disque rippé en entier) : les lecteurs audio s'en servent
pour jouer toutes les pistes au même volume (−18 LUFS). Le son n'est pas
modifié. Désactivable dans les réglages (ou `--no-replaygain`) ; pas de tags
en WAV.

## Piste cachée et pré-accentuation

L'audio placé avant la piste 1 (HTOA, « hidden track one audio ») est rippé en
piste 00, sauf s'il n'est que du silence ; le fichier cue le place avant la
piste 1, comme sur le disque. Les pistes enregistrées avec pré-accentuation
(certains CD des années 80) sont gardées telles quelles, signalées dans le
cue (`FLAGS PRE`) et le log : c'est au lecteur audio de corriger.

## Organisation

```
crates/
  galette-core/        moteur, sans aucune interface
    src/toc.rs         table des matières (pistes, lead-out, CD Extra)
    src/ids.rs         identifiants : MusicBrainz Disc ID, FreeDB, AccurateRip
    src/rip.rs         extraction d'une piste : décalage, mode sécurisé, C2, relectures
    src/accuraterip.rs sommes de contrôle v1/v2, base de décalages des lecteurs
    src/flac.rs        encodage FLAC (vérifié par décodage), tags et pochette intégrés
    src/mp3.rs         encodage MP3 avec LAME (VBR V0/V2 ou CBR 320, lecture sans blanc)
    src/alac.rs        encodage ALAC / Apple Lossless (vérifié par décodage)
    src/aac.rs         encodage AAC par l'encodeur d'Apple (macOS, lecture sans blanc)
    src/m4a.rs         conteneur MPEG-4 (.m4a)
    src/aiff.rs        écriture des fichiers AIFF
    src/tags.rs        tags au format de MusicBrainz Picard (Vorbis, ID3v2.4, iTunes)
    src/gaps.rs        détection des pré-gaps par le sous-canal Q
    src/offset.rs      recherche du décalage du lecteur avec un disque connu d'AccurateRip
    src/cache.rs       mesure du cache du lecteur
    src/replaygain.rs  mesure du volume perçu (EBU R128) pour les tags ReplayGain
    src/cue.rs         fichier .cue (un fichier par piste, pré-gaps en fin de piste précédente)
    src/log.rs         log de rip
    src/wav.rs         écriture des fichiers WAV
    src/drive/         accès au lecteur
      mmc.rs           commandes MMC (INQUIRY, MODE SENSE, READ TOC, READ CD, READ SUB-CHANNEL),
                       communes à tous les OS
      linux.rs         envoi des commandes via l'ioctl SG_IO
      macos.rs         envoi des commandes via IOKit (SCSITask), démontage du disque
      windows.rs       envoi des commandes via IOCTL_SCSI_PASS_THROUGH_DIRECT
    src/metadata/      métadonnées (MusicBrainz) et pochettes (Cover Art Archive)
    src/naming.rs      noms des dossiers et fichiers à partir de modèles ({albumartist}, {track}…)
    src/session.rs     déroulé complet d'un rip, partagé par toutes les interfaces
  galette-cli/         interface en ligne de commande
  galette-gui/         application graphique (Tauri)
    src/main.rs        commandes appelées par la fenêtre, rip en arrière-plan,
                       surveillance du lecteur, réglages (settings.json)
    ui/                la fenêtre : HTML, CSS et JavaScript, sans framework
```

Le moteur ne dépend d'aucune interface : la ligne de commande et l'application
graphique l'utilisent de la même manière. Pour porter Galette
sur un nouvel OS, il suffit d'implémenter le trait `drive::Transport`, qui envoie
une commande SCSI au lecteur.

## Feuille de route

- [x] Lecture de la TOC (Linux), Disc ID MusicBrainz, FreeDB, AccurateRip
- [x] Recherche de l'album sur MusicBrainz
- [x] Extraction des pistes en WAV
- [x] Correction du décalage de lecture (base de décalages AccurateRip)
- [x] Vérification AccurateRip (CRC v1 / v2)
- [x] Mode sécurisé : relectures, purge du cache, erreurs C2
- [x] Encodage FLAC + tags + pochette (Cover Art Archive)
- [x] Fichier `.cue` (pré-gaps, ISRC, MCN), log de rip
- [x] Application graphique (Tauri) : édition des métadonnées, choix de l'édition, progression
- [x] Détection automatique de l'insertion d'un disque, réglages (nommage, décalage, C2, pochette, éjection)
- [x] Paquets d'installation Linux (.deb, AppImage)
- [x] macOS (IOKit), application .app / .dmg
- [x] MP3 (LAME) et AIFF, tags ID3
- [x] ALAC (Apple Lossless), tags iTunes
- [x] AAC (macOS, encodeur d'Apple, 256 kbit/s VBR)
- [x] Windows (`IOCTL_SCSI_PASS_THROUGH_DIRECT`), installateur NSIS
- [x] CD-Text : titres lus sur le disque quand MusicBrainz ne le connaît pas
- [x] Vérification CTDB (CUETools Database), en plus d'AccurateRip
- [x] Réparation CTDB : les pistes abîmées sont corrigées avec les données de
  parité de la base, aidées par les erreurs C2 signalées par le lecteur
- [x] Piste cachée avant la piste 1 (HTOA), pré-accentuation signalée
- [x] Recherche du décalage d'un lecteur inconnu, test du cache du lecteur
- [x] ReplayGain 2.0 (piste et album)
