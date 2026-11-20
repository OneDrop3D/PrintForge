// Banc de l'interface dans WebKit, sans l'application : ui/ est servi en HTTP et window.__TAURI__ remplacé par un faux
// pont qui répond selon un scénario. Vérifie chaque vue, la copie automatique sur clé USB et le bandeau de mise à jour.
// Lancer : node tests/ui.mjs   (BANC = dossier du banc Playwright de OneDrop3D ; CAPTURES = dossier des captures, facultatif)
import { createRequire } from 'node:module';
import { spawn } from 'node:child_process';
import assert from 'node:assert/strict';
import net from 'node:net';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const banc = process.env.BANC || process.env.HOME + '/Library/Application Support/fr.onedrop3d.gestion/banc-maquette';
process.env.PLAYWRIGHT_BROWSERS_PATH ||= banc + '/navigateurs';
const { webkit } = createRequire(banc + '/')('playwright');
const ui = path.join(path.dirname(fileURLToPath(import.meta.url)), '..', 'ui');

// Un port libre (le 9333 de ce Mac est souvent pris), puis le serveur de fichiers de Python.
const port = await new Promise((ok) => { const s = net.createServer().listen(0, () => { const p = s.address().port; s.close(() => ok(p)); }); });
const serveur = spawn('python3', ['-m', 'http.server', String(port), '--bind', '127.0.0.1', '--directory', ui], { stdio: 'ignore' });
const url = `http://127.0.0.1:${port}/index.html`;
for (let i = 0; i < 50; i++) { try { await fetch(url); break; } catch { await new Promise((r) => setTimeout(r, 100)); } }

/** Le faux pont Tauri : chaque commande renvoie window.__s[commande] ({ erreur } la fait échouer). */
function pont(scenario) {
  window.__appels = [];
  window.__s = scenario;
  const repondre = (cmd) => {
    if (!(cmd in window.__s)) throw `commande inattendue : ${cmd}`;
    const r = window.__s[cmd];
    if (r && r.erreur) throw r.erreur;
    return structuredClone(r);
  };
  window.__TAURI__ = {
    core: { invoke: async (cmd, args = {}) => { window.__appels.push([cmd, args]); return repondre(cmd); } },
    dialog: {
      open: async (o) => { window.__appels.push(['dialog.open', o]); return window.__s.dialogue_fichier; },
      ask: async (texte) => { window.__appels.push(['dialog.ask', texte]); return window.__s.dialogue_oui; }
    },
    opener: { openUrl: async (u) => { window.__appels.push(['openUrl', u]); } },
    app: { getVersion: async () => '0.1.0' }
  };
}

const version = (source, v, beta, nom) => ({ source, version: v, beta, nom, taille: 10, sha256: 'ab',
  url: `https://github.com/${source === 'Prusa' ? 'prusa3d' : 'OneDrop3D'}/Prusa-Firmware-Buddy/releases/download/${v}/${nom}` });
const VERSIONS = [version('OneDrop3D', 'od3d-8.0.3', true, 'COREONE_firmware_8.0.3-OD3D.bbf'),
  version('OneDrop3D', 'od3d-8.0.2', false, 'COREONE_firmware_8.0.2-OD3D.bbf'),
  version('Prusa', 'v8.0.2', false, 'COREONE_COREONE+GEN2_firmware_8.0.2.bbf')];
const cle = (nom, probleme = '') => ({ chemin: `/Volumes/${nom}`, nom, probleme });
const BASE = {
  imprimante_config: { adresse: '192.168.1.50', motDePasse: true },
  imprimante_etat: { version: { hostname: 'coreone-atelier', firmware: '8.0.2-OD3D+11843', printer: '7.1.0' },
    statut: { printer: { state: 'PRINTING', temp_nozzle: 215.04, target_nozzle: 215, temp_bed: 59.9, target_bed: 60 },
      job: { id: 3, progress: 42.4, time_remaining: 3900, time_printing: 1200 } } },
  maj_verifier: null,
  firmware_versions: VERSIONS,
  firmware_cles: [cle('CLE-A')],
  firmware_copier: { nom: 'x.bbf', ejectee: true },
  dialogue_oui: true
};

const erreurs = [];
const navigateur = await webkit.launch();
async function ouvrir(scenario, sombre = false) {
  const contexte = await navigateur.newContext({ viewport: { width: 1000, height: 720 }, colorScheme: sombre ? 'dark' : 'light' });
  const p = await contexte.newPage();
  p.on('pageerror', (e) => erreurs.push(String(e)));
  p.on('console', (m) => { if (m.type() === 'error') erreurs.push(m.text()); });
  await p.addInitScript(pont, { ...BASE, ...scenario });
  await p.goto(url);
  return p;
}
const texte = (p, s) => p.locator(s).innerText();
const appels = (p, cmd) => p.evaluate((c) => window.__appels.filter(([x]) => x === c).map(([, a]) => a), cmd);
const changer = (p, valeurs) => p.evaluate((v) => Object.assign(window.__s, v), valeurs);
const attendreAppel = (p, cmd, n = 1) =>
  p.waitForFunction(([c, k]) => window.__appels.filter(([x]) => x === c).length >= k, [cmd, n], { timeout: 6000 });
async function capture(p, nom) { if (process.env.CAPTURES) await p.screenshot({ path: path.join(process.env.CAPTURES, nom + '.png') }); }
const resultats = [];
async function essai(nom, f) { await f(); resultats.push(nom); console.log('ok  ' + nom); }

try {
  await essai('Imprimante : état lu et traduit', async () => {
    const p = await ouvrir({});
    await p.waitForFunction(() => document.querySelector('#eEtat').textContent);
    assert.equal(await texte(p, '#eNom'), 'coreone-atelier');
    assert.equal(await texte(p, '#eFirmware'), '8.0.2-OD3D+11843');
    assert.equal(await texte(p, '#eEtat'), 'Impression en cours');
    assert.equal(await texte(p, '#eBuse'), '215 °C → 215 °C');
    assert.equal(await texte(p, '#ePlateau'), '59,9 °C → 60 °C');
    assert.equal(await texte(p, '#eTravail'), '42 %, encore 1 h 05');
    assert.equal(await p.inputValue('#adresse'), '192.168.1.50');
    assert.match(await p.getAttribute('#motDePasse', 'placeholder'), /Enregistré/);
    assert.ok(await p.locator('#bandeau').isHidden(), 'pas de bandeau sans mise à jour');
    assert.equal(await p.getAttribute('#onglet-imprimante', 'aria-selected'), 'true');
    await capture(p, 'imprimante');
    await changer(p, { imprimante_etat: { erreur: 'Mot de passe refusé : recopiez celui affiché sur l’imprimante.' } });
    await p.waitForFunction(() => document.querySelector('#msgEtat').textContent.includes('Mot de passe refusé'), null, { timeout: 6000 });
    assert.match(await p.getAttribute('#msgEtat', 'class'), /ko/);
    await p.context().close();
  });

  await essai('Imprimante : recherche Bonjour puis enregistrement', async () => {
    const p = await ouvrir({ imprimante_config: { adresse: '', motDePasse: false }, imprimantes_chercher: [{ nom: 'Prusa CORE One', adresse: '192.168.1.77' }],
      imprimante_enregistrer: { hostname: 'coreone', firmware: '8.0.2' } });
    await p.waitForFunction(() => window.__appels.some(([c]) => c === 'imprimante_config'));
    assert.ok(await p.locator('#sansImprimante').isVisible());
    assert.ok(await p.locator('#etat').isHidden());
    await p.click('#chercher');
    await p.click('#trouvees button');
    assert.equal(await p.inputValue('#adresse'), '192.168.1.77');
    await p.fill('#motDePasse', 'secret123');
    await changer(p, { imprimante_config: { adresse: '192.168.1.77', motDePasse: true } });
    await p.click('#enregistrer');
    await p.waitForFunction(() => document.querySelector('#msgRelier').className.includes('ok'));
    assert.deepEqual((await appels(p, 'imprimante_enregistrer'))[0], { adresse: '192.168.1.77', motDePasse: 'secret123' });
    assert.equal(await p.inputValue('#motDePasse'), '', 'le mot de passe ne reste pas dans le champ');
    await attendreAppel(p, 'imprimante_etat');
    assert.equal((await appels(p, 'imprimante_etat')).length >= 1, true);
    await p.context().close();
  });

  await essai('Firmware : versions, clé déjà là, clé branchée, clé refusée', async () => {
    const p = await ouvrir({});
    await p.click('#onglet-firmware');
    await p.waitForFunction(() => document.querySelectorAll('#version option').length === 3);
    assert.deepEqual(await p.$$eval('#version optgroup', (g) => g.map((x) => x.label)), ['OneDrop3D', 'Officiel Prusa']);
    assert.deepEqual(await p.$$eval('#version option', (o) => o.map((x) => x.textContent)), ['od3d-8.0.3 (bêta)', 'od3d-8.0.2', 'v8.0.2']);
    // une clé déjà branchée attend le bouton
    await p.waitForFunction(() => document.querySelector('#cle').textContent.includes('CLE-A'), null, { timeout: 6000 });
    assert.equal(await texte(p, '#cle'), 'Clé « CLE-A » branchée.');
    assert.ok(await p.locator('#copier').isVisible());
    await p.waitForTimeout(2300);
    assert.equal((await appels(p, 'firmware_copier')).length, 0, 'pas de copie sans geste pour une clé déjà là');
    // une clé branchée pendant que la vue est affichée est préparée tout de suite, avec la version choisie
    await p.selectOption('#version', '1');
    await changer(p, { firmware_cles: [cle('CLE-A'), cle('CLE-B')] });
    await attendreAppel(p, 'firmware_copier');
    const copie = (await appels(p, 'firmware_copier'))[0];
    assert.equal(copie.cle, '/Volumes/CLE-B');
    assert.equal(copie.version.version, 'od3d-8.0.2');
    await p.waitForFunction(() => document.querySelector('#msgCle').textContent.includes('éjectée'));
    assert.equal(await texte(p, '#msgCle'), 'od3d-8.0.2 est sur la clé, qui est éjectée : branchez-la sur l’imprimante.');
    // une clé qui ne convient pas : la raison, pas de bouton, pas de copie
    await changer(p, { firmware_cles: [cle('CLE-A'), cle('CLE-B'), cle('CLE-C', 'elle est en ExFAT, et l’imprimante ne lit que le FAT32')] });
    await p.waitForFunction(() => document.querySelector('#cle').textContent.includes('CLE-C'), null, { timeout: 6000 });
    assert.equal(await texte(p, '#cle'), 'La clé « CLE-C » ne convient pas : elle est en ExFAT, et l’imprimante ne lit que le FAT32. Reformatez-la (voir plus bas).');
    assert.ok(await p.locator('#copier').isHidden());
    assert.equal((await appels(p, 'firmware_copier')).length, 1);
    await capture(p, 'firmware');
    // vue quittée puis rouverte : les clés présentes ne partent pas seules ; le bouton copie sur la première
    await p.click('#onglet-envoyer');
    await p.waitForTimeout(2300);
    await changer(p, { firmware_cles: [cle('CLE-A'), cle('CLE-D')] });
    await p.click('#onglet-firmware');
    await p.waitForFunction(() => document.querySelector('#cle').textContent === 'Clé « CLE-A » branchée.', null, { timeout: 6000 });
    await p.waitForTimeout(2300);
    assert.equal((await appels(p, 'firmware_copier')).length, 1, 'la clé D, branchée vue fermée, attend le bouton');
    await p.click('#copier');
    await attendreAppel(p, 'firmware_copier', 2);
    assert.equal((await appels(p, 'firmware_copier'))[1].cle, '/Volumes/CLE-A');
    // Windows : pas d'éjection, la page le demande
    await changer(p, { firmware_copier: { nom: 'x.bbf', ejectee: null } });
    await p.click('#copier');
    await p.waitForFunction(() => document.querySelector('#msgCle').textContent.includes('Retirer le périphérique'));
    await p.context().close();
  });

  await essai('Firmware : installation par le réseau', async () => {
    const p = await ouvrir({ firmware_reseau: { erreur: 'Le firmware de votre imprimante ne permet pas encore l’installation par le réseau : utilisez une clé USB (ci-dessus).' } });
    await p.click('#onglet-firmware');
    await p.waitForFunction(() => document.querySelectorAll('#version option').length === 3);
    await changer(p, { dialogue_oui: false });
    await p.click('#reseau');
    await p.waitForTimeout(200);
    assert.equal((await appels(p, 'dialog.ask')).length, 1, 'confirmation demandée');
    assert.equal((await appels(p, 'firmware_reseau')).length, 0, 'rien sans accord');
    await changer(p, { dialogue_oui: true });
    await p.click('#reseau');
    await p.waitForFunction(() => document.querySelector('#msgReseau').className.includes('ko'));
    assert.match(await texte(p, '#msgReseau'), /ne permet pas encore/);
    assert.equal((await appels(p, 'firmware_reseau'))[0].version.version, 'od3d-8.0.3');
    await changer(p, { firmware_reseau: { firmware: '8.0.3-OD3D+11900' } });
    await p.click('#reseau');
    await p.waitForFunction(() => document.querySelector('#msgReseau').className.includes('ok'));
    assert.equal(await texte(p, '#msgReseau'), 'Installé : l’imprimante a maintenant le firmware 8.0.3-OD3D+11900.');
    await p.click('[data-lien]');
    assert.match((await appels(p, 'openUrl'))[0], /^https:\/\/help\.prusa3d\.com\//);
    await p.context().close();
  });

  await essai('Envoyer : G-code pas lancé (409), puis musique', async () => {
    const p = await ouvrir({ dialogue_fichier: '/Users/moi/Benchy.bgcode', envoyer_fichier: { nom: 'Benchy.bgcode', lance: false }, imprimer: null,
      musique_envoyer: { nom: 'musique-Tetris.gcode', lance: true, notes: 120, duree: 95 } });
    await p.click('#onglet-envoyer');
    assert.ok(await p.locator('#envoyerGcode').isDisabled());
    await p.click('#choisirGcode');
    assert.equal(await texte(p, '#nomGcode'), 'Benchy.bgcode');
    assert.deepEqual((await appels(p, 'dialog.open'))[0].filters, [{ name: 'G-code', extensions: ['gcode', 'bgcode'] }]);
    await p.click('#envoyerGcode');
    await p.waitForFunction(() => !document.querySelector('#lancerGcode').hidden);
    assert.deepEqual((await appels(p, 'envoyer_fichier'))[0], { chemin: '/Users/moi/Benchy.bgcode' });
    assert.match(await texte(p, '#msgGcode'), /ne peut pas le lancer maintenant/);
    await p.click('#lancerGcode');
    await p.waitForFunction(() => document.querySelector('#msgGcode').textContent.includes('est lancé'));
    assert.deepEqual((await appels(p, 'imprimer'))[0], { nom: 'Benchy.bgcode' });
    assert.ok(await p.locator('#lancerGcode').isHidden());
    await changer(p, { dialogue_fichier: '/Users/moi/Tetris.mid' });
    await p.click('#choisirMidi');
    await p.fill('#pas', '80');
    await p.click('#envoyerMidi');
    await p.waitForFunction(() => document.querySelector('#msgMidi').className.includes('ok'));
    assert.deepEqual((await appels(p, 'musique_envoyer'))[0], { chemin: '/Users/moi/Tetris.mid', pasParMm: 80, jouer: true });
    assert.equal(await texte(p, '#msgMidi'), '« musique-Tetris.gcode » (120 notes, 2 min) est sur l’imprimante : c’est parti.');
    await p.fill('#pas', '0');
    await p.click('#envoyerMidi');
    assert.match(await texte(p, '#msgMidi'), /positif/);
    assert.equal((await appels(p, 'musique_envoyer')).length, 1);
    await capture(p, 'envoyer');
    await p.context().close();
  });

  await essai('Mise à jour : bandeau, installation, hors ligne, à jour', async () => {
    const notes = '# OneDrop PrintForge 0.2.0 — nouveautés\n\n- **Musique** : plus fidèle.\n- **Firmware** : installation par le réseau.';
    const p = await ouvrir({ maj_verifier: { version: '0.2.0', notes, page: 'https://github.com/OneDrop3D/mises-a-jour/releases' },
      maj_installer: '/Users/moi/Downloads/OneDrop-PrintForge-0.2.0-mac-arm64.dmg' }, true);
    await p.waitForSelector('#bandeau:not([hidden])');
    assert.equal(await texte(p, '#majTitre'), 'OneDrop PrintForge 0.2.0 est disponible.');
    assert.equal(await p.locator('#majNotes').textContent(), '- Musique : plus fidèle.\n- Firmware : installation par le réseau.');
    await p.click('#majInstaller');
    await p.waitForFunction(() => document.querySelector('#majMessage').className.includes('ok'));
    assert.match(await texte(p, '#majMessage'), /^OneDrop-PrintForge-0\.2\.0-mac-arm64\.dmg est téléchargé et vérifié/);
    await capture(p, 'mise-a-jour-sombre');
    await p.click('#majFermer');
    assert.ok(await p.locator('#bandeau').isHidden());
    await p.context().close();

    const horsLigne = await ouvrir({ maj_verifier: { erreur: 'Pas de connexion à GitHub (io: hôte introuvable).' } });
    await horsLigne.waitForFunction(() => window.__appels.some(([c]) => c === 'maj_verifier'));
    await horsLigne.waitForTimeout(200);
    assert.ok(await horsLigne.locator('#bandeau').isHidden(), 'hors ligne au démarrage : rien');
    await horsLigne.click('#majVerifier');
    await horsLigne.waitForFunction(() => document.querySelector('#majTitre').textContent.startsWith('Vérification impossible'));
    assert.ok(await horsLigne.locator('#majInstaller').isHidden());
    await changer(horsLigne, { maj_verifier: null });
    await horsLigne.click('#majVerifier');
    await horsLigne.waitForFunction(() => document.querySelector('#majTitre').textContent.includes('à jour'));
    assert.equal(await texte(horsLigne, '#majTitre'), 'OneDrop PrintForge est à jour (version 0.1.0).');
    await horsLigne.context().close();
  });

  assert.deepEqual(erreurs, [], 'erreurs JavaScript dans la page');
  console.log(`\n${resultats.length} essais réussis, aucune erreur dans la page.`);
} catch (e) {
  console.error('ÉCHEC :', e.message, erreurs.length ? `\nerreurs de la page : ${erreurs.join(' | ')}` : '');
  process.exitCode = 1;
} finally {
  await navigateur.close();
  serveur.kill();
}
