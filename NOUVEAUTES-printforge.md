<!--
  NOUVEAUTÉS À PUBLIER — fichier lu par le Publieur OneDrop3D.
  POUR L'IA DE CODE : quand tu termines des modifications sur cette application, ajoute ici,
  sous ce commentaire, une ligne par point visible par l'utilisateur (Markdown, en français,
  ex. « - **Réglages** : le thème sombre s'applique aussi à la fenêtre d'aide »).
  Mets aussi à jour le README / la documentation si une fonction change.
  Ne mets pas de numéro de version : le Publieur le gère. Il vide ce fichier après la publication.
-->
- **Firmware du MMU3** : sur CORE One, la carte indique d’activer d’abord « Préparer la mise à jour USB du MMU » sur l’imprimante (OneDrop3D 8.0.2.27 ou plus récent), puis de toucher « Terminer » après l’installation.
- **Firmware du MMU3** : l’installation n’échoue plus avec « Le MMU ne répond plus (Operation timed out) » : la présentation au chargeur du MMU suit celle d’avrdude (une commande à la fois, nouvel essai si le chargeur ne répond pas encore), et un message d’erreur nomme la commande restée sans réponse.
- **Firmware du MMU3** : un MMU que l’ordinateur ne voit pas s’installe quand même, comme dans PrusaSlicer : « Installer sur le MMU » demande d’appuyer sur le bouton reset du MMU et attend son mode de mise à jour 30 s.
