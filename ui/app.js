/* OneDrop PrintForge : trois vues (Imprimante, Firmware, Envoyer) et le bandeau des mises à jour.
   Tout ce qui touche l’imprimante, GitHub ou les disques passe par les commandes Rust (src-tauri/src). */
const T = window.__TAURI__, invoke = T.core.invoke, $ = (s) => document.querySelector(s);
const memoire = {
  lire(k) { try { return localStorage.getItem(k); } catch { return null; } },
  ecrire(k, v) { try { localStorage.setItem(k, v); } catch {} }
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
const ETATS = { IDLE: 'Au repos', READY: 'Prête', BUSY: 'Occupée', PRINTING: 'Impression en cours', PAUSED: 'En pause',
  FINISHED: 'Impression terminée', STOPPED: 'Impression arrêtée', ERROR: 'En erreur', ATTENTION: 'Attend une action sur son écran' };
const degres = (mesure, cible) =>
  `${Number(mesure ?? 0).toLocaleString('fr-FR', { maximumFractionDigits: 1 })} °C` + (cible ? ` → ${Math.round(cible)} °C` : '');

let reliee = false;
async function lireConfig(remplir) {
  const c = await invoke('imprimante_config');
  if (remplir) $('#adresse').value = c.adresse;
  $('#motDePasse').placeholder = c.motDePasse ? 'Enregistré (laissez vide pour le garder)' : '';
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
    const p = s.printer || {}, j = s.job;
    $('#eNom').textContent = v.hostname || 'CORE One';
    $('#eFirmware').textContent = v.firmware || '?';
    $('#eEtat').textContent = ETATS[p.state] || p.state || '?';
    $('#eBuse').textContent = degres(p.temp_nozzle, p.target_nozzle);
    $('#ePlateau').textContent = degres(p.temp_bed, p.target_bed);
    for (const e of document.querySelectorAll('.travail')) e.hidden = !j;
    if (j) $('#eTravail').textContent = `${Math.round(j.progress ?? 0)} %` + (j.time_remaining != null ? `, encore ${duree(j.time_remaining)}` : '');
    $('#etat').hidden = false;
    dire($('#msgEtat'), '');
  } catch (e) { dire($('#msgEtat'), String(e), 'ko'); }
  finally { lecture = false; }
}
setInterval(lireEtat, 4000);

$('#chercher').onclick = async (ev) => {
  const m = $('#msgChercher');
  ev.target.disabled = true; dire(m, 'Recherche sur le réseau…'); $('#trouvees').replaceChildren();
  try {
    const liste = await invoke('imprimantes_chercher');
    dire(m, liste.length ? 'Choisissez votre imprimante, puis saisissez son mot de passe.'
      : 'Aucune imprimante trouvée. Vérifiez qu’elle est allumée et sur le même réseau ; sur Mac, autorisez OneDrop PrintForge dans Réglages Système › Confidentialité et sécurité › Réseau local. Vous pouvez aussi saisir son adresse IP, affichée dans Réglages › Réseau sur l’imprimante.');
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
  ev.target.disabled = true; dire(m, 'Connexion à l’imprimante…');
  try {
    const v = await invoke('imprimante_enregistrer', { adresse: $('#adresse').value, motDePasse: $('#motDePasse').value });
    $('#motDePasse').value = '';
    dire(m, `Imprimante reliée : ${v.hostname || 'CORE One'}, firmware ${v.firmware || '?'}.`, 'ok');
  } catch (e) { dire(m, String(e), 'ko'); }
  finally { ev.target.disabled = false; await lireConfig(false); lireEtat(); }
};

/* ---------- Firmware ---------- */
// Les versions de l'imprimante ou du MMU dans leur liste : les nôtres, puis les officielles.
let versions = [], versionsMmu = [];
async function lireVersions(commande, liste, m, pour) {
  let lues;
  try { lues = await invoke(commande); }
  catch (e) {
    liste.replaceChildren(new Option('Versions illisibles'));
    dire(m, `Impossible de lire les versions sur GitHub : ${e}`, 'ko');
    return [];
  }
  liste.replaceChildren(...[['OneDrop3D', 'OneDrop3D'], ['Prusa', 'Officiel Prusa']].map(([source, titre]) => {
    const groupe = document.createElement('optgroup');
    groupe.label = titre;
    lues.forEach((v, i) => { if (v.source === source) groupe.append(new Option(v.version + (v.beta ? ' (bêta)' : ''), i)); });
    return groupe;
  }));
  if (!lues.length) dire(m, `Aucune version n’est publiée pour ${pour}.`, 'ko');
  return lues;
}
const versionChoisie = () => versions[$('#version').value];

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
    lireVersions('firmware_versions', $('#version'), $('#msgVersions'), 'la CORE One'),
    lireVersions('mmu_versions', $('#versionMmu'), $('#msgMmu'), 'le MMU3'),
  ]);
  for (const b of boutons) b.disabled = false;
  const heure = new Date().toLocaleTimeString('fr-FR', { hour: '2-digit', minute: '2-digit' });
  if (versions.length) dire($('#msgVersions'), `Liste lue à ${heure}.`);
  if (versionsMmu.length) dire($('#msgMmu'), `Liste lue à ${heure}.`);
}
$('#actualiserVersions').onclick = actualiserVersions;
$('#actualiserMmu').onclick = actualiserVersions;

let occupe = false;
async function copier(cle) {
  const v = versionChoisie(), m = $('#msgCle');
  if (!v) return dire(m, 'Les versions ne sont pas encore lues : rebranchez la clé dans un instant.', 'ko');
  occupe = true; $('#copier').disabled = true;
  dire(m, `Téléchargement de ${v.version} sur « ${cle.nom} »…`);
  try {
    const r = await invoke('firmware_copier', { cle: cle.chemin, version: v });
    dire(m, r.ejectee ? `${v.version} est sur la clé, qui est éjectée : branchez-la sur l’imprimante.`
      : r.ejectee === false ? `${v.version} est sur la clé, mais elle n’a pas pu être éjectée : éjectez-la avant de la retirer.`
      : `${v.version} est sur la clé : éjectez-la (« Retirer le périphérique en toute sécurité »), puis branchez-la sur l’imprimante.`, 'ok');
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
  $('#cle').textContent = !cle ? 'Branchez une clé USB.'
    : cle.probleme ? `La clé « ${cle.nom} » ne convient pas : ${cle.probleme}. Reformatez-la (voir plus bas).` : `Clé « ${cle.nom} » branchée.`;
  $('#copier').hidden = !cle || !!cle.probleme;
  $('#copier').onclick = () => copier(cle);
  if (nouvelle) { dire($('#msgCle'), ''); if (!nouvelle.probleme) copier(nouvelle); }
}
setInterval(surveiller, 2000);

$('#reseau').onclick = async (ev) => {
  const v = versionChoisie(), m = $('#msgReseau');
  if (!v) return dire(m, 'Les versions ne sont pas encore lues.', 'ko');
  const oui = await T.dialog.ask(`Installer ${v.version} par le réseau ? L’imprimante va redémarrer pour l’installer : ne l’éteignez pas pendant l’installation.`,
    { title: 'OneDrop PrintForge', kind: 'warning', okLabel: 'Installer', cancelLabel: 'Annuler' });
  if (!oui) return;
  ev.target.disabled = true; dire(m, `Envoi de ${v.version} à l’imprimante, puis installation : jusqu’à quelques minutes…`);
  try {
    const r = await invoke('firmware_reseau', { version: v });
    dire(m, `Installé : l’imprimante a maintenant le firmware ${r.firmware}.`, 'ok');
  } catch (e) { dire(m, String(e), 'ko'); }
  finally { ev.target.disabled = false; }
};

/* ---------- MMU3 ---------- */
const ETATS_MMU = { absent: 'Aucun MMU relié à cet ordinateur.', firmware: 'MMU3 relié.',
  chargeur: 'MMU3 relié, dans son mode de mise à jour.', plusieurs: 'Plusieurs MMU sont reliés : n’en gardez qu’un.' };
let flashMmu = false;
async function surveillerMmu() {
  if (!visible('firmware') || flashMmu) return;
  let etat;
  try { etat = await invoke('mmu_detecter'); } catch { return; }
  $('#mmu').textContent = ETATS_MMU[etat];
  $('#flasherMmu').hidden = etat !== 'firmware' && etat !== 'chargeur';
}
setInterval(surveillerMmu, 2000);

$('#flasherMmu').onclick = async (ev) => {
  const v = versionsMmu[$('#versionMmu').value], m = $('#msgMmu'), barre = $('#progresMmu');
  if (!v) return dire(m, 'Les versions ne sont pas encore lues.', 'ko');
  const oui = await T.dialog.ask(`Installer ${v.version} sur le MMU ? Ne débranchez ni le câble USB ni l’imprimante pendant l’installation.`,
    { title: 'OneDrop PrintForge', kind: 'warning', okLabel: 'Installer', cancelLabel: 'Annuler' });
  if (!oui) return;
  flashMmu = true; ev.target.disabled = true; barre.value = 0; barre.hidden = false;
  const fin = await T.event.listen('mmu', ({ payload: p }) => {
    if (p.texte) dire(m, p.texte);
    if (p.total) barre.value = p.fait / p.total;
  });
  try {
    const r = await invoke('mmu_flasher', { version: v });
    dire(m, `${v.version} est installé et relu sans erreur${r.redemarre ? ', et le MMU a redémarré' : ''}. Débranchez le câble USB. Si l’imprimante affiche une erreur du MMU, redémarrez-la : elle relira sa version (Info › Infos Version).`, 'ok');
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
  try { await invoke('imprimer', { nom }); bouton.hidden = true; dire(m, `« ${nom} » est lancé.`, 'ok'); }
  catch (e) { dire(m, String(e), 'ko'); }
  finally { bouton.disabled = false; }
}

// 409 « Can't print right now » : le fichier est sur l'imprimante mais pas lancé ; le bouton le lancera plus tard.
async function envoyer(bouton, m, boutonLancer, commande, args, lancerAussitot) {
  bouton.disabled = true; boutonLancer.hidden = true;
  dire(m, 'Envoi à l’imprimante…');
  try {
    const r = await invoke(commande, args);
    const detail = r.notes ? ` (${r.notes} notes, ${duree(r.duree)})` : '';
    if (r.lance) dire(m, `« ${r.nom} »${detail} est sur l’imprimante : c’est parti.`, 'ok');
    else if (lancerAussitot) dire(m, `« ${r.nom} »${detail} est sur l’imprimante, mais elle ne peut pas le lancer maintenant (impression en cours, ou action attendue sur son écran).`, 'ko');
    else dire(m, `« ${r.nom} »${detail} est sur l’imprimante.`, 'ok');
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
  if (!(pas > 0)) return dire(m, 'Le nombre de pas par millimètre doit être un nombre positif.', 'ko');
  const cles = await invoke('firmware_cles').catch(() => []);
  const cle = cles.find((c) => !c.probleme);
  if (!cle) return dire(m, cles.length ? `La clé « ${cles[0].nom} » ne convient pas : ${cles[0].probleme}.` : 'Branchez une clé USB sur cet ordinateur.', 'ko');
  ev.target.disabled = true;
  dire(m, `Écriture sur la clé « ${cle.nom} »…`);
  try {
    const r = await invoke('musique_cle', { chemin: midi, pasParMm: pas, cle: cle.chemin });
    const fait = `« ${r.nom} » (${r.notes} notes, ${duree(r.duree)}) est sur la clé`;
    dire(m, r.ejectee ? `${fait}, qui est éjectée : branchez-la sur l’imprimante et lancez le fichier depuis Imprimer.`
      : r.ejectee === false ? `${fait}, mais elle n’a pas pu être éjectée : éjectez-la avant de la retirer.`
      : `${fait} : éjectez-la, puis branchez-la sur l’imprimante et lancez le fichier depuis Imprimer.`, 'ok');
  } catch (e) { dire(m, String(e), 'ko'); }
  finally { ev.target.disabled = false; }
};
$('#envoyerMidi').onclick = (ev) => {
  const pas = Number($('#pas').value), jouer = $('#jouer').checked;
  if (!(pas > 0)) return dire($('#msgMidi'), 'Le nombre de pas par millimètre doit être un nombre positif.', 'ko');
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
  if (manuel) bandeau('Recherche d’une mise à jour…');
  try {
    const m = await invoke('maj_verifier');
    if (m) bandeau(`OneDrop PrintForge ${m.version} est disponible.`, String(m.notes || '').replace(/^#.*\n+/, '').replace(/\*\*/g, ''), true);
    else if (manuel) bandeau(`OneDrop PrintForge est à jour${versionApp ? ` (version ${versionApp})` : ''}.`);
  } catch (e) { if (manuel) bandeau(`Vérification impossible : ${e}`); }
}
$('#majVerifier').onclick = () => verifierMaj(true);
$('#majFermer').onclick = () => { $('#bandeau').hidden = true; };
$('#majInstaller').onclick = async (ev) => {
  const m = $('#majMessage');
  ev.target.disabled = true; dire(m, 'Téléchargement de la mise à jour…');
  try {
    const chemin = await invoke('maj_installer');
    dire(m, `${nomDe(chemin)} est téléchargé et vérifié, et s’ouvre. Sur Mac, glissez OneDrop PrintForge dans Applications pour remplacer cette version ; sur Windows, suivez l’installateur.`, 'ok');
  } catch (e) { dire(m, String(e), 'ko'); ev.target.disabled = false; }
};

/* ---------- Démarrage ---------- */
T.app.getVersion().then((v) => { versionApp = v; $('#versionApp').textContent = ` · ${v}`; }).catch(() => {});
ouvrir(VUES.find((v) => v === memoire.lire('pf-vue')) || 'imprimante');
lireConfig(true).then(lireEtat).catch((e) => dire($('#msgEtat'), String(e), 'ko'));
if (!versionsLues) actualiserVersions(); // déjà lancée si l'application rouvre sur l'onglet Firmware
verifierMaj(false);
