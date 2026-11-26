/* OneDrop PrintForge : trois vues (Imprimante, Firmware, Envoyer) et le bandeau des mises à jour.
   Tout ce qui touche l’imprimante, GitHub ou les disques passe par les commandes Rust (src-tauri/src).
   En français ou en anglais : les textes anglais de la page sont dans ses attributs data-en*, ceux des messages dans L(),
   ceux du code Rust dans tr! (commande « langue »). */
const T = window.__TAURI__, invoke = T.core.invoke, $ = (s) => document.querySelector(s);
const memoire = {
  lire(k) { try { return localStorage.getItem(k); } catch { return null; } },
  ecrire(k, v) { try { localStorage.setItem(k, v); } catch {} }
};
/* ---------- Langue ---------- */
// Celle de l'ordinateur (français, sinon anglais), ou celle choisie par le bouton FR / EN de l'en-tête
const anglais = (memoire.lire('pf-langue') || navigator.language || 'en').slice(0, 2).toLowerCase() !== 'fr';
const L = (fr, en) => (anglais ? en : fr), locale = L('fr-FR', 'en-GB');
const cite = (t) => L(`« ${t} »`, `“${t}”`);
if (anglais) {
  document.documentElement.lang = 'en';
  for (const e of document.querySelectorAll('[data-en]')) e.innerHTML = e.dataset.en;
  for (const a of ['placeholder', 'title', 'aria-label'])
    for (const e of document.querySelectorAll(`[data-en-${a}]`)) e.setAttribute(a, e.getAttribute(`data-en-${a}`));
}
$('#langue').onclick = () => {
  if (occupe || flashMmu) return; // la page rechargée perdrait le suivi de la copie ou du flash en cours
  memoire.ecrire('pf-langue', L('en', 'fr'));
  location.reload();
};

function dire(e, texte, genre = '') { e.textContent = texte; e.className = 'message ' + genre; }
const visible = (vue) => !document.hidden && !$('#' + vue).hidden;
const nomDe = (chemin) => String(chemin).split(/[\\/]/).pop();
function duree(s) {
  if (s < 60) return `${Math.round(s)} s`;
  const h = Math.floor(s / 3600), m = Math.round((s % 3600) / 60);
  return h ? `${h} h ${String(m).padStart(2, '0')}` : `${m} min`;
}

/* ---------- Vues ---------- */
const VUES = ['imprimante', 'firmware', 'envoyer'];
function ouvrir(vue) {
  for (const b of document.querySelectorAll('[data-vue]')) b.setAttribute('aria-selected', String(b.dataset.vue === vue));
  for (const v of VUES) $('#' + v).hidden = v !== vue;
  memoire.ecrire('pf-vue', vue);
  lireEtat();
  // Une nouvelle version a pu sortir depuis l'ouverture de l'application : liste relue si elle a plus de 5 min
  if (vue === 'firmware' && Date.now() - versionsLues > 5 * 60e3) actualiserVersions();
}
for (const b of document.querySelectorAll('[data-vue]')) b.onclick = () => ouvrir(b.dataset.vue);

for (const b of document.querySelectorAll('[data-lien]'))
  b.onclick = async () => {
    try { await T.opener.openUrl(b.dataset.lien); }
    catch { b.textContent = b.dataset.lien; }  // le lien qui ne s'ouvre pas montre son adresse, pour la recopier
  };

/* ---------- Imprimante ---------- */
const ETATS = anglais
  ? { IDLE: 'Idle', READY: 'Ready', BUSY: 'Busy', PRINTING: 'Printing', PAUSED: 'Paused', FINISHED: 'Print finished',
    STOPPED: 'Print stopped', ERROR: 'Error', ATTENTION: 'Waiting for an action on its screen' }
  : { IDLE: 'Au repos', READY: 'Prête', BUSY: 'Occupée', PRINTING: 'Impression en cours', PAUSED: 'En pause',
    FINISHED: 'Impression terminée', STOPPED: 'Impression arrêtée', ERROR: 'En erreur', ATTENTION: 'Attend une action sur son écran' };
const degres = (mesure, cible) =>
  `${Number(mesure ?? 0).toLocaleString(locale, { maximumFractionDigits: 1 })} °C` + (cible ? ` → ${Math.round(cible)} °C` : '');

let reliee = false;
async function lireConfig(remplir) {
  const c = await invoke('imprimante_config');
  if (remplir) $('#adresse').value = c.adresse;
  $('#motDePasse').placeholder = c.motDePasse ? L('Enregistré (laissez vide pour le garder)', 'Saved (leave empty to keep it)') : '';
  reliee = !!c.adresse;
  $('#sansImprimante').hidden = reliee;
}

// État relu toutes les 4 s, seulement quand la vue est affichée.
let lecture = false;
async function lireEtat() {
  if (!reliee || lecture || !visible('imprimante')) return;
  lecture = true;
  try {
    const { version: v, statut: s } = await invoke('imprimante_etat');
    if (!memoire.lire('pf-modele')) choisirModele(v.modele); // sinon, le modèle choisi à la main reste
    const p = s.printer || {}, j = s.job;
    $('#eNom').textContent = v.hostname || 'CORE One';
    $('#eFirmware').textContent = v.firmware || '?';
    $('#eEtat').textContent = ETATS[p.state] || p.state || '?';
    $('#eBuse').textContent = degres(p.temp_nozzle, p.target_nozzle);
    $('#ePlateau').textContent = degres(p.temp_bed, p.target_bed);
    for (const e of document.querySelectorAll('.travail')) e.hidden = !j;
    if (j) $('#eTravail').textContent = `${Math.round(j.progress ?? 0)}${L(' %', '%')}`
      + (j.time_remaining != null ? L(`, encore ${duree(j.time_remaining)}`, `, ${duree(j.time_remaining)} left`) : '');
    $('#etat').hidden = false;
    dire($('#msgEtat'), '');
  } catch (e) { dire($('#msgEtat'), String(e), 'ko'); }
  finally { lecture = false; }
}
setInterval(lireEtat, 4000);

$('#chercher').onclick = async (ev) => {
  const m = $('#msgChercher');
  ev.target.disabled = true; dire(m, L('Recherche sur le réseau…', 'Searching the network…')); $('#trouvees').replaceChildren();
  try {
    const liste = await invoke('imprimantes_chercher');
    dire(m, liste.length ? L('Choisissez votre imprimante, puis saisissez son mot de passe.', 'Choose your printer, then enter its code or password.')
      : L('Aucune imprimante trouvée. Vérifiez qu’elle est allumée et sur le même réseau ; sur Mac, autorisez OneDrop PrintForge dans Réglages Système › Confidentialité et sécurité › Réseau local. Vous pouvez aussi saisir son adresse IP, affichée dans Réglages › Réseau sur l’imprimante.',
        'No printer found. Check that it is on and on the same network; on Mac, allow OneDrop PrintForge in System Settings › Privacy & Security › Local Network. You can also enter its IP address, shown in Settings › Network on the printer.'));
    $('#trouvees').replaceChildren(...liste.map((t) => {
      const b = document.createElement('button');
      b.className = 'bouton'; b.textContent = `${t.nom} (${t.adresse})`;
      b.onclick = () => { $('#adresse').value = t.adresse; $('#motDePasse').focus(); };
      return b;
    }));
  } catch (e) { dire(m, String(e), 'ko'); }
  finally { ev.target.disabled = false; }
};

$('#enregistrer').onclick = async (ev) => {
  const m = $('#msgRelier');
  ev.target.disabled = true; dire(m, L('Connexion à l’imprimante…', 'Connecting to the printer…'));
  try {
    const v = await invoke('imprimante_enregistrer', { adresse: $('#adresse').value, motDePasse: $('#motDePasse').value });
    $('#motDePasse').value = '';
    choisirModele(v.modele); // la liste des firmwares passe à cette imprimante
    dire(m, L('Imprimante reliée : ', 'Printer linked: ') + `${v.hostname || 'CORE One'}, firmware ${v.firmware || '?'}.`, 'ok');
  } catch (e) { dire(m, String(e), 'ko'); }
  finally { ev.target.disabled = false; await lireConfig(false); lireEtat(); }
};

/* ---------- Firmware ---------- */
// Les versions de l'imprimante ou du MMU dans leur liste : les nôtres, puis les officielles.
let versions = [], versionsMmu = [];
// garder : pour l'imprimante, les versions du modèle choisi ; pour le MMU, toutes
function remplir(liste, lues, garder = () => true) {
  liste.replaceChildren(...[['OneDrop3D', 'OneDrop3D'], ['Prusa', L('Officiel Prusa', 'Official Prusa')]].map(([source, titre]) => {
    const groupe = document.createElement('optgroup');
    groupe.label = titre;
    lues.forEach((v, i) => { if (v.source === source && garder(v)) groupe.append(new Option(v.version + (v.beta ? L(' (bêta)', ' (beta)') : ''), i)); });
    return groupe;
  }).filter((g) => g.children.length));
}
async function lireVersions(commande, liste, m, pour, garder) {
  let lues;
  try { lues = await invoke(commande); }
  catch (e) {
    liste.replaceChildren(new Option(L('Versions illisibles', 'Unreadable versions')));
    dire(m, L(`Impossible de lire les versions sur GitHub : ${e}`, `Could not read the versions on GitHub: ${e}`), 'ko');
    return [];
  }
  remplir(liste, lues, garder);
  if (!lues.length) dire(m, L(`Aucune version n’est publiée pour ${pour}.`, `No version is published for ${pour}.`), 'ko');
  return lues;
}
const versionChoisie = () => versions[$('#version').value];

// Imprimante : chaque modèle a son firmware. Le choix est gardé ; l'imprimante qu'on relie le remplace.
const modeles = [...$('#modele').options].map((o) => o.value);
const duModele = (v) => v.modele === $('#modele').value;
// Les versions du modèle choisi, sans relire GitHub, ou pourquoi il n'y en a pas
function afficherModele(lue = '') {
  if (!versions.length) return;
  remplir($('#version'), versions, duModele);
  const nom = $('#modele').selectedOptions[0].textContent;
  if ($('#version').options.length) dire($('#msgVersions'), lue);
  else dire($('#msgVersions'), L(`Aucune version n’est publiée pour la ${nom}.`, `No version is published for the ${nom}.`), 'ko');
}
function choisirModele(m) {
  if (!modeles.includes(m) || m === $('#modele').value) return;
  $('#modele').value = m;
  memoire.ecrire('pf-modele', m);
  afficherModele();
}
$('#modele').value = modeles.includes(memoire.lire('pf-modele')) ? memoire.lire('pf-modele') : 'coreone';
$('#modele').onchange = () => { memoire.ecrire('pf-modele', $('#modele').value); afficherModele(); };

// Les deux listes relues ensemble (bouton « Actualiser la liste », ou onglet ouvert après 5 min) ; jamais pendant une
// copie ou un flash, qui gardent la version choisie au départ
let versionsLues = 0;
async function actualiserVersions() {
  if (occupe || flashMmu) return;
  versionsLues = Date.now();
  const boutons = [$('#actualiserVersions'), $('#actualiserMmu')];
  for (const b of boutons) b.disabled = true;
  dire($('#msgVersions'), ''); dire($('#msgMmu'), '');
  [versions, versionsMmu] = await Promise.all([
    lireVersions('firmware_versions', $('#version'), $('#msgVersions'), L('la CORE One', 'the CORE One'), duModele),
    lireVersions('mmu_versions', $('#versionMmu'), $('#msgMmu'), L('le MMU3', 'the MMU3')),
  ]);
  for (const b of boutons) b.disabled = false;
  const heure = new Date().toLocaleTimeString(locale, { hour: '2-digit', minute: '2-digit' });
  const lue = L(`Liste lue à ${heure}.`, `List read at ${heure}.`);
  afficherModele(lue);
  if (versionsMmu.length) dire($('#msgMmu'), lue);
}
$('#actualiserVersions').onclick = actualiserVersions;
$('#actualiserMmu').onclick = actualiserVersions;

let occupe = false;
async function copier(cle) {
  const v = versionChoisie(), m = $('#msgCle');
  if (!v) return dire(m, L('Les versions ne sont pas encore lues : rebranchez la clé dans un instant.', 'The versions are not read yet: plug the drive in again in a moment.'), 'ko');
  occupe = true; $('#copier').disabled = true;
  dire(m, L(`Téléchargement de ${v.version} sur ${cite(cle.nom)}…`, `Downloading ${v.version} to ${cite(cle.nom)}…`));
  try {
    const r = await invoke('firmware_copier', { cle: cle.chemin, version: v });
    dire(m, v.version + (r.ejectee ? L(' est sur la clé, qui est éjectée : branchez-la sur l’imprimante.', ' is on the drive, which is ejected: plug it into the printer.')
      : r.ejectee === false ? L(' est sur la clé, mais elle n’a pas pu être éjectée : éjectez-la avant de la retirer.', ' is on the drive, but it could not be ejected: eject it before removing it.')
      : L(' est sur la clé : éjectez-la (« Retirer le périphérique en toute sécurité »), puis branchez-la sur l’imprimante.', ' is on the drive: eject it (“Safely Remove Hardware”), then plug it into the printer.')), 'ok');
  } catch (e) { dire(m, String(e), 'ko'); }
  finally { occupe = false; $('#copier').disabled = false; }
}

// Une clé branchée pendant que la vue est affichée est préparée tout de suite ; une clé déjà là attend le bouton.
let connues = null;
async function surveiller() {
  if (!visible('firmware')) { connues = null; return; }
  if (occupe) return;
  let cles;
  try { cles = await invoke('firmware_cles'); } catch { return; }
  const nouvelle = connues && cles.find((c) => !connues.includes(c.chemin));
  connues = cles.map((c) => c.chemin);
  const cle = nouvelle || cles[0];
  $('#cle').textContent = !cle ? L('Branchez une clé USB.', 'Plug in a USB drive.')
    : cle.probleme ? L(`La clé ${cite(cle.nom)} ne convient pas : ${cle.probleme}. Reformatez-la (voir plus bas).`, `The drive ${cite(cle.nom)} won’t do: ${cle.probleme}. Reformat it (see below).`)
    : L(`Clé ${cite(cle.nom)} branchée.`, `Drive ${cite(cle.nom)} plugged in.`);
  $('#copier').hidden = !cle || !!cle.probleme;
  $('#copier').onclick = () => copier(cle);
  if (nouvelle) { dire($('#msgCle'), ''); if (!nouvelle.probleme) copier(nouvelle); }
}
setInterval(surveiller, 2000);

$('#reseau').onclick = async (ev) => {
  const v = versionChoisie(), m = $('#msgReseau');
  if (!v) return dire(m, L('Les versions ne sont pas encore lues.', 'The versions are not read yet.'), 'ko');
  const oui = await T.dialog.ask(L(`Installer ${v.version} par le réseau ? L’imprimante va redémarrer pour l’installer : ne l’éteignez pas pendant l’installation.`,
    `Install ${v.version} over the network? The printer will restart to install it: don’t turn it off during the installation.`),
    { title: 'OneDrop PrintForge', kind: 'warning', okLabel: L('Installer', 'Install'), cancelLabel: L('Annuler', 'Cancel') });
  if (!oui) return;
  ev.target.disabled = true; dire(m, L(`Envoi de ${v.version} à l’imprimante, puis installation : jusqu’à quelques minutes…`, `Sending ${v.version} to the printer, then installing: up to a few minutes…`));
  try {
    const r = await invoke('firmware_reseau', { version: v });
    dire(m, L(`Installé : l’imprimante a maintenant le firmware ${r.firmware}.`, `Installed: the printer now has firmware ${r.firmware}.`), 'ok');
  } catch (e) { dire(m, String(e), 'ko'); }
  finally { ev.target.disabled = false; }
};

/* ---------- MMU3 ---------- */
const ETATS_MMU = anglais
  ? { absent: 'No MMU seen. If it is connected, “Install on the MMU” will ask you to press its reset button.', firmware: 'MMU3 connected.',
    chargeur: 'MMU3 connected, in its update mode.', plusieurs: 'Several MMUs are connected: keep only one.' }
  : { absent: 'Aucun MMU vu. S’il est relié, « Installer sur le MMU » vous demandera d’appuyer sur son bouton reset.', firmware: 'MMU3 relié.',
    chargeur: 'MMU3 relié, dans son mode de mise à jour.', plusieurs: 'Plusieurs MMU sont reliés : n’en gardez qu’un.' };
let flashMmu = false;
async function surveillerMmu() {
  if (!visible('firmware') || flashMmu) return;
  let etat;
  try { etat = await invoke('mmu_detecter'); } catch { return; }
  $('#mmu').textContent = ETATS_MMU[etat];
  $('#flasherMmu').hidden = etat === 'plusieurs'; // sans MMU vu aussi : mode de secours par le bouton reset
}
setInterval(surveillerMmu, 2000);

$('#flasherMmu').onclick = async (ev) => {
  const v = versionsMmu[$('#versionMmu').value], m = $('#msgMmu'), barre = $('#progresMmu');
  if (!v) return dire(m, L('Les versions ne sont pas encore lues.', 'The versions are not read yet.'), 'ko');
  const oui = await T.dialog.ask(L(`Installer ${v.version} sur le MMU ? Ne débranchez ni le câble USB ni l’imprimante pendant l’installation.`,
    `Install ${v.version} on the MMU? Don’t unplug the USB cable or the printer during the installation.`),
    { title: 'OneDrop PrintForge', kind: 'warning', okLabel: L('Installer', 'Install'), cancelLabel: L('Annuler', 'Cancel') });
  if (!oui) return;
  flashMmu = true; ev.target.disabled = true; barre.value = 0; barre.hidden = false;
  const fin = await T.event.listen('mmu', ({ payload: p }) => {
    if (p.texte) dire(m, p.texte);
    if (p.total) barre.value = p.fait / p.total;
  });
  try {
    const r = await invoke('mmu_flasher', { version: v });
    dire(m, anglais
      ? `${v.version} is installed and read back without error${r.redemarre ? ', and the MMU restarted' : ''}. Unplug the USB cable. If the printer shows an MMU error, restart it: it will read its version again (Info › Version Info).`
      : `${v.version} est installé et relu sans erreur${r.redemarre ? ', et le MMU a redémarré' : ''}. Débranchez le câble USB. Si l’imprimante affiche une erreur du MMU, redémarrez-la : elle relira sa version (Info › Infos Version).`, 'ok');
  } catch (e) { dire(m, String(e), 'ko'); }
  finally { fin(); flashMmu = false; ev.target.disabled = false; barre.hidden = true; }
};

/* ---------- Envoyer ---------- */
async function choisir(nom, extensions, etiquette, bouton) {
  const chemin = await T.dialog.open({ multiple: false, directory: false, filters: [{ name: nom, extensions }] });
  if (chemin) { etiquette.textContent = nomDe(chemin); bouton.disabled = false; }
  return chemin;
}

async function lancer(bouton, m, nom) {
  bouton.disabled = true;
  try { await invoke('imprimer', { nom }); bouton.hidden = true; dire(m, cite(nom) + L(' est lancé.', ' has started.'), 'ok'); }
  catch (e) { dire(m, String(e), 'ko'); }
  finally { bouton.disabled = false; }
}

// 409 « Can't print right now » : le fichier est sur l'imprimante mais pas lancé ; le bouton le lancera plus tard.
async function envoyer(bouton, m, boutonLancer, commande, args, lancerAussitot) {
  bouton.disabled = true; boutonLancer.hidden = true;
  dire(m, L('Envoi à l’imprimante…', 'Sending to the printer…'));
  try {
    const r = await invoke(commande, args);
    const fichier = cite(r.nom) + (r.notes ? ` (${r.notes} notes, ${duree(r.duree)})` : '');
    if (r.lance) dire(m, fichier + L(' est sur l’imprimante : c’est parti.', ' is on the printer: here it goes.'), 'ok');
    else if (lancerAussitot) dire(m, fichier + L(' est sur l’imprimante, mais elle ne peut pas le lancer maintenant (impression en cours, ou action attendue sur son écran).',
      ' is on the printer, but it can’t start it now (print in progress, or an action is awaited on its screen).'), 'ko');
    else dire(m, fichier + L(' est sur l’imprimante.', ' is on the printer.'), 'ok');
    if (!r.lance) { boutonLancer.hidden = false; boutonLancer.onclick = () => lancer(boutonLancer, m, r.nom); }
  } catch (e) { dire(m, String(e), 'ko'); }
  finally { bouton.disabled = false; }
}

let gcode = null, midi = null;
$('#choisirGcode').onclick = async () => {
  const c = await choisir('G-code', ['gcode', 'bgcode'], $('#nomGcode'), $('#envoyerGcode'));
  if (c) { gcode = c; $('#lancerGcode').hidden = true; dire($('#msgGcode'), ''); }
};
$('#envoyerGcode').onclick = (ev) => envoyer(ev.target, $('#msgGcode'), $('#lancerGcode'), 'envoyer_fichier', { chemin: gcode }, true);

// 100 était l'ancienne valeur par défaut, en micro-pas : 6,25 pas complets donnent la même note, en 16 fois plus grand
$('#pas').value = (memoire.lire('pf-pas') === '100' ? null : memoire.lire('pf-pas')) || 6.25;
$('#pas').onchange = () => memoire.ecrire('pf-pas', $('#pas').value);
$('#choisirMidi').onclick = async () => {
  const c = await choisir('MIDI', ['mid', 'midi'], $('#nomMidi'), $('#envoyerMidi'));
  if (c) { midi = c; $('#lancerMidi').hidden = true; $('#cleMidi').disabled = false; dire($('#msgMidi'), ''); }
};
// Sans réseau : le G-code de musique sur une clé USB de cet ordinateur, la première que l'imprimante pourra lire.
$('#cleMidi').onclick = async (ev) => {
  const pas = Number($('#pas').value), m = $('#msgMidi');
  if (!(pas > 0)) return dire(m, L('Le nombre de pas par millimètre doit être un nombre positif.', 'Steps per millimetre must be a positive number.'), 'ko');
  const cles = await invoke('firmware_cles').catch(() => []);
  const cle = cles.find((c) => !c.probleme);
  if (!cle) return dire(m, cles.length ? L(`La clé ${cite(cles[0].nom)} ne convient pas : ${cles[0].probleme}.`, `The drive ${cite(cles[0].nom)} won’t do: ${cles[0].probleme}.`)
    : L('Branchez une clé USB sur cet ordinateur.', 'Plug a USB drive into this computer.'), 'ko');
  ev.target.disabled = true;
  dire(m, L(`Écriture sur la clé ${cite(cle.nom)}…`, `Writing to the drive ${cite(cle.nom)}…`));
  try {
    const r = await invoke('musique_cle', { chemin: midi, pasParMm: pas, cle: cle.chemin });
    const fait = `${cite(r.nom)} (${r.notes} notes, ${duree(r.duree)})` + L(' est sur la clé', ' is on the drive');
    dire(m, fait + (r.ejectee ? L(', qui est éjectée : branchez-la sur l’imprimante et lancez le fichier depuis Imprimer.', ', which is ejected: plug it into the printer and start the file from Print.')
      : r.ejectee === false ? L(', mais elle n’a pas pu être éjectée : éjectez-la avant de la retirer.', ', but it could not be ejected: eject it before removing it.')
      : L(' : éjectez-la, puis branchez-la sur l’imprimante et lancez le fichier depuis Imprimer.', ': eject it, then plug it into the printer and start the file from Print.')), 'ok');
  } catch (e) { dire(m, String(e), 'ko'); }
  finally { ev.target.disabled = false; }
};
$('#envoyerMidi').onclick = (ev) => {
  const pas = Number($('#pas').value), jouer = $('#jouer').checked;
  if (!(pas > 0)) return dire($('#msgMidi'), L('Le nombre de pas par millimètre doit être un nombre positif.', 'Steps per millimetre must be a positive number.'), 'ko');
  envoyer(ev.target, $('#msgMidi'), $('#lancerMidi'), 'musique_envoyer', { chemin: midi, pasParMm: pas, jouer }, jouer);
};

/* ---------- Mises à jour de l'application ---------- */
let versionApp = '';
function bandeau(titre, notes = '', installer = false) {
  $('#majTitre').textContent = titre;
  $('#majNotes').textContent = notes;
  $('#majDetails').hidden = !notes;
  $('#majInstaller').hidden = !installer;
  $('#majInstaller').disabled = false;
  dire($('#majMessage'), '');
  $('#bandeau').hidden = false;
}
// Au démarrage, rien ne s'affiche sans mise à jour (manifeste pas encore publié, hors ligne) ; le bouton, lui, répond toujours.
async function verifierMaj(manuel) {
  if (manuel) bandeau(L('Recherche d’une mise à jour…', 'Looking for an update…'));
  try {
    const m = await invoke('maj_verifier');
    if (m) bandeau(L(`OneDrop PrintForge ${m.version} est disponible.`, `OneDrop PrintForge ${m.version} is available.`), String(m.notes || '').replace(/^#.*\n+/, '').replace(/\*\*/g, ''), true);
    else if (manuel) bandeau(L('OneDrop PrintForge est à jour', 'OneDrop PrintForge is up to date') + (versionApp ? ` (version ${versionApp}).` : '.'));
  } catch (e) { if (manuel) bandeau(L(`Vérification impossible : ${e}`, `Could not check: ${e}`)); }
}
$('#majVerifier').onclick = () => verifierMaj(true);
$('#majFermer').onclick = () => { $('#bandeau').hidden = true; };
$('#majInstaller').onclick = async (ev) => {
  const m = $('#majMessage');
  ev.target.disabled = true; dire(m, L('Téléchargement de la mise à jour…', 'Downloading the update…'));
  try {
    const chemin = await invoke('maj_installer');
    dire(m, nomDe(chemin) + L(' est téléchargé et vérifié, et s’ouvre. Sur Mac, glissez OneDrop PrintForge dans Applications pour remplacer cette version ; sur Windows, suivez l’installateur.',
      ' is downloaded and verified, and is opening. On Mac, drag OneDrop PrintForge into Applications to replace this version; on Windows, follow the installer.'), 'ok');
  } catch (e) { dire(m, String(e), 'ko'); ev.target.disabled = false; }
};

/* ---------- Démarrage ---------- */
T.app.getVersion().then((v) => { versionApp = v; $('#versionApp').textContent = ` · ${v}`; }).catch(() => {});
// Le code Rust apprend la langue avant tout autre appel : ses messages arrivent dans celle de la page
invoke('langue', { anglais }).catch(() => {}).then(() => {
  ouvrir(VUES.find((v) => v === memoire.lire('pf-vue')) || 'imprimante');
  lireConfig(true).then(lireEtat).catch((e) => dire($('#msgEtat'), String(e), 'ko'));
  if (!versionsLues) actualiserVersions(); // déjà lancée si l'application rouvre sur l'onglet Firmware
  verifierMaj(false);
});
