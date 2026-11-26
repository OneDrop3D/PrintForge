# OneDrop PrintForge — pour Prusa CORE One

Application gratuite pour les propriétaires d'une Prusa CORE One qui utilisent le **firmware OneDrop3D** ([OneDrop3D/Prusa-Firmware-Buddy](https://github.com/OneDrop3D/Prusa-Firmware-Buddy)). Ce n'est pas un produit Prusa Research.

- **Imprimante** : trouve l'imprimante sur le réseau local (PrusaLink) et affiche son état.
- **Firmware** : liste les versions OneDrop3D et officielles de votre imprimante (CORE One, One+ et One+ Gen2, CORE One L, et leurs versions INDX ; l’imprimante reliée est reconnue toute seule), puis les installe par le réseau (firmware OneDrop3D 8.0.2.2 ou plus récent) ou en préparant une clé USB toute seule.
- **Envoyer** : envoie un G-code et lance l'impression ; transforme un fichier MIDI en musique jouée par les moteurs.
- **Mises à jour** : l'application se met à jour elle-même.
- **Langues** : français ou anglais, selon la langue de l'ordinateur ; le bouton FR / EN de l'en-tête change de langue.

Le premier firmware OneDrop3D s'installe à la main : il faut casser une fois l'« appendice » de la carte xBuddy (geste irréversible, voir l'article de Prusa).

## Nouveautés de la version 0.1.0
- Première version : imprimante, firmware, envoi de fichiers et musique.

## Construire
`./build.sh` (Mac, application signée), `./build.sh mac` (installeur .dmg dans `dist/`). Windows : workflow GitHub `build.yml`.
