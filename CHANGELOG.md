# Journal des versions

## Version 0.1.10

- **Firmware** : un choix « Imprimante » au-dessus de la liste des versions (CORE One, One+ et One+ Gen2 ; CORE One L ; CORE One INDX ; CORE One L INDX). Chaque modèle reçoit son propre firmware. L’imprimante reliée est reconnue toute seule, le choix est gardé pour préparer la clé d’une autre imprimante, et l’installation par le réseau refuse un firmware fait pour un autre modèle.
- **Anglais** : l’application existe aussi en anglais. Elle prend la langue de l’ordinateur (français sur un ordinateur en français, anglais sinon), et le bouton FR / EN de l’en-tête change de langue. Les messages d’erreur, la demande d’accès au réseau local sur Mac et l’installateur Windows suivent aussi la langue.

---

## Version 0.1.8

- **Firmware du MMU3** : sur CORE One, la carte indique d’activer d’abord « Préparer la mise à jour USB du MMU » sur l’imprimante (OneDrop3D 8.0.2.27 ou plus récent), puis de toucher « Terminer » après l’installation.
- **Firmware du MMU3** : l’installation n’échoue plus avec « Le MMU ne répond plus (Operation timed out) » : la présentation au chargeur du MMU suit celle d’avrdude (une commande à la fois, nouvel essai si le chargeur ne répond pas encore), et un message d’erreur nomme la commande restée sans réponse.
- **Firmware du MMU3** : un MMU que l’ordinateur ne voit pas s’installe quand même, comme dans PrusaSlicer : « Installer sur le MMU » demande d’appuyer sur le bouton reset du MMU et attend son mode de mise à jour 30 s.

---

## Version 0.1.7

- **Installation sur Mac** : le fichier .dmg montre l'application à côté d'un raccourci « Applications » ; il suffit de glisser l'une sur l'autre, sans ouvrir de Finder.
- **Firmware** : un bouton « Actualiser la liste » relit les versions de la CORE One et du MMU3 sur GitHub ; la liste se relit aussi toute seule en ouvrant l'onglet Firmware si elle a plus de 5 minutes.
- **Firmware du MMU3** : installation du firmware du MMU par le port USB de sa carte, version OneDrop3D ou officielle Prusa, vérifiée avant et après l'écriture.
- **Windows** : l'installeur Windows est de nouveau construit.

---

## Version 0.1.0
- Première version : connexion à l'imprimante par PrusaLink, liste et installation du firmware (réseau ou clé USB), envoi de G-code, musique MIDI jouée par les moteurs, mises à jour automatiques.
