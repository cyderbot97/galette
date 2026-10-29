# Galette

Ripper de CD audio **sécurisé**, **libre** (GPL-3.0) et **multiplateforme** (Linux, Windows, macOS).

L'objectif : la fiabilité d'EAC et la simplicité de dBpoweramp CD Ripper.
On insère un CD, Galette le reconnaît sur MusicBrainz, extrait l'audio sans erreur,
vérifie le résultat avec AccurateRip et produit des fichiers FLAC correctement tagués.

> État : début de développement. Sous Linux, Galette reconnaît le CD sur MusicBrainz,
> l'extrait avec correction du décalage du lecteur, vérifie chaque piste avec
> AccurateRip et produit des FLAC tagués avec leur pochette. Pas encore de mode sécurisé.

## Essayer

Il faut [Rust](https://rustup.rs).

```sh
cargo run -p galette-cli -- info                    # CD dans /dev/sr0
cargo run -p galette-cli -- info --device /dev/sr1
cargo run --release -p galette-cli -- rip -o ~/Musique             # FLAC + tags + pochette
cargo run --release -p galette-cli -- rip -o ~/Musique --format wav
# Sans lecteur, à partir d'une TOC au format MusicBrainz :
cargo run -p galette-cli -- info --toc "1 12 242387 150 21636 50551 70783 90795 113395 133171 142126 159578 181123 198419 218045"
cargo test
```

## Organisation

```
crates/
  galette-core/        moteur, sans aucune interface
    src/toc.rs         table des matières (pistes, lead-out, CD Extra)
    src/ids.rs         identifiants : MusicBrainz Disc ID, FreeDB, AccurateRip
    src/rip.rs         extraction d'une piste avec correction du décalage
    src/accuraterip.rs sommes de contrôle v1/v2, base de décalages des lecteurs
    src/flac.rs        encodage FLAC (vérifié par décodage), tags et pochette intégrés
    src/tags.rs        tags au format de MusicBrainz Picard
    src/wav.rs         écriture des fichiers WAV
    src/drive/         accès au lecteur
      mmc.rs           commandes MMC (READ TOC, READ CD), communes à tous les OS
      linux.rs         envoi des commandes via l'ioctl SG_IO
    src/metadata/      métadonnées (MusicBrainz) et pochettes (Cover Art Archive)
  galette-cli/         interface en ligne de commande
```

Le moteur ne dépend d'aucune interface : la ligne de commande et la future
application graphique (Tauri) l'utilisent de la même manière. Pour porter Galette
sur un nouvel OS, il suffit d'implémenter le trait `drive::Transport`, qui envoie
une commande SCSI au lecteur.

## Feuille de route

- [x] Lecture de la TOC (Linux), Disc ID MusicBrainz, FreeDB, AccurateRip
- [x] Recherche de l'album sur MusicBrainz
- [x] Extraction des pistes en WAV
- [x] Correction du décalage de lecture (base de décalages AccurateRip)
- [x] Vérification AccurateRip (CRC v1 / v2)
- [ ] Mode sécurisé : relectures, purge du cache, erreurs C2
- [x] Encodage FLAC + tags + pochette (Cover Art Archive)
- [ ] Fichier `.cue`, log de rip
- [ ] Application graphique (Tauri)
- [ ] Windows (`IOCTL_SCSI_PASS_THROUGH_DIRECT`), macOS (IOKit)
- [ ] CTDB (vérification et réparation), pré-gaps, HTOA, CD-Text
