/* Le firmware du MMU3 : les versions publiées sur GitHub, puis le flash par le port USB de la carte du MMU, comme
 * PrusaSlicer (src/slic3r/GUI/FirmwareDialog.cpp) : le MMU (0x2c99/4) ouvert à 1200 bauds redémarre dans son
 * chargeur Caterina (0x2c99/3), qui parle AVR109 ; les commandes sont celles d’avrdude -c avr109 -p atmega32u4 -D. */
use crate::firmware::{fichier, versions};
use crate::telecharger;
use serde_json::{json, Value};
use serialport::SerialPortType;
use std::io::{Read, Write};
use std::thread::sleep;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

const DEPOTS: [(&str, &str); 2] = [("OneDrop3D", "OneDrop3D/Prusa-Firmware-MMU"), ("Prusa", "prusa3d/Prusa-Firmware-MMU")];
const PRUSA: u16 = 0x2c99;
const CHARGEUR: u16 = 3;
const FIRMWARE: u16 = 4;
/// 28 672 octets pour le firmware : les 4 Ko du haut de l’ATmega32U4 sont au chargeur.
const APPLI: usize = 0x7000;
const PAGE: usize = 128;
const REPRISE: &str = "Le MMU n’est pas abîmé : son chargeur reste joignable. Relancez l’installation ; si le MMU n’est plus détecté, éteignez puis rallumez l’imprimante (elle l’alimente), puis relancez.";

/// Le .hex du MMU3 d’une release (« MMU3_FW3.0.4+936.hex », « MMU2S_MMU3_FW3.0.4+940.hex »).
fn mmu3(_: &str, nom: &str) -> bool {
    nom.contains("MMU3") && nom.ends_with(".hex")
}

#[tauri::command(async)]
pub fn mmu_versions() -> Result<Vec<Value>, String> {
    versions(&DEPOTS, mmu3)
}

/// Les MMU reliés à cet ordinateur : (port, 3 pour le chargeur ou 4 pour le firmware).
fn mmus() -> Vec<(String, u16)> {
    serialport::available_ports()
        .unwrap_or_default()
        .into_iter()
        .filter(|p| !p.port_name.starts_with("/dev/tty.")) // macOS donne chaque port deux fois : cu. et tty.
        .filter_map(|p| match p.port_type {
            SerialPortType::UsbPort(u) if u.vid == PRUSA && [CHARGEUR, FIRMWARE].contains(&u.pid) => Some((p.port_name, u.pid)),
            _ => None,
        })
        .collect()
}

#[tauri::command(async)]
pub fn mmu_detecter() -> &'static str {
    match mmus()[..] {
        [] => "absent",
        [(_, CHARGEUR)] => "chargeur",
        [_] => "firmware",
        _ => "plusieurs",
    }
}

/// Le seul MMU relié avec ce numéro de produit, attendu jusqu’à `secondes` (sondé toutes les 500 ms, comme PrusaSlicer).
fn attendre(pid: u16, secondes: u64) -> Option<String> {
    let fin = Instant::now() + Duration::from_secs(secondes);
    while Instant::now() < fin {
        sleep(Duration::from_millis(500));
        if let [(port, p)] = &mmus()[..] {
            if *p == pid {
                return Some(port.clone());
            }
        }
    }
    None
}

fn dire(app: &AppHandle, texte: &str) {
    let _ = app.emit("mmu", json!({ "texte": texte }));
}

/// Le port du chargeur : le MMU qui tourne y est renvoyé en ouvrant son port à 1200 bauds (avr109_reboot de PrusaSlicer).
fn chargeur(app: &AppHandle) -> Result<String, String> {
    let port = match &mmus()[..] {
        [] => return Err("Aucun MMU relié : reliez le port micro-USB de la carte du MMU à cet ordinateur, imprimante allumée, avec un câble qui transmet les données.".into()),
        [(port, CHARGEUR)] => return Ok(port.clone()),
        [(port, _)] => port.clone(),
        _ => return Err("Plusieurs MMU sont reliés à cet ordinateur : n’en gardez qu’un.".into()),
    };
    dire(app, "Redémarrage du MMU dans son mode de mise à jour…");
    if let Ok(p) = serialport::new(&port, 1200).open() {
        sleep(Duration::from_millis(50));
        drop(p);
    }
    if let Some(port) = attendre(CHARGEUR, 5) {
        return Ok(port);
    }
    dire(app, "Le MMU ne redémarre pas seul : appuyez sur le bouton reset de sa carte, à côté du port USB.");
    attendre(CHARGEUR, 20).ok_or_else(|| "Le MMU n’est pas passé dans son mode de mise à jour : rien n’a été écrit. Vérifiez le câble, puis réessayez.".into())
}

/// Le firmware vérifié écrit dans le MMU, relu, puis le MMU redémarré.
#[tauri::command(async)]
pub fn mmu_flasher(app: AppHandle, version: Value) -> Result<Value, String> {
    let (url, _, taille, sha256) = fichier(&DEPOTS, mmu3, &version)?;
    if taille > 256 * 1024 {
        return Err("Ce fichier est trop gros pour être un firmware du MMU3.".into());
    }
    dire(&app, "Téléchargement et vérification du firmware…");
    let octets = telecharger(&url, taille, &sha256)?;
    let (image, pages) = lire_hex(std::str::from_utf8(&octets).map_err(|_| "Ce fichier n’est pas un .hex lisible.".to_string())?)?;
    let port = chargeur(&app)?;
    dire(&app, "Écriture du firmware, puis relecture : ne débranchez rien…");
    // le port du chargeur peut mettre un instant à s’ouvrir après son apparition
    let mut ouvert = Err("".to_string());
    for _ in 0..5 {
        ouvert = serialport::new(&port, 57600).timeout(Duration::from_secs(3)).open().map_err(|e| e.to_string());
        if ouvert.is_ok() {
            break;
        }
        sleep(Duration::from_millis(400));
    }
    let mut p = ouvert.map_err(|e| format!("Le port du MMU ne s’ouvre pas ({e}). {REPRISE}"))?;
    let _ = p.clear(serialport::ClearBuffer::Input);
    flasher(&mut p, &image, &pages, |fait, total| {
        let _ = app.emit("mmu", json!({ "fait": fait, "total": total }));
    })
    .map_err(|e| format!("{e} {REPRISE}"))?;
    drop(p);
    dire(&app, "Firmware écrit et vérifié : redémarrage du MMU…");
    Ok(json!({ "redemarre": attendre(FIRMWARE, 10).is_some() }))
}

/// L’image du firmware lue dans le .hex (Intel HEX, jusqu’au premier enregistrement de fin comme PrusaSlicer) :
/// 0xFF dans les trous, et les pages de 128 octets qui portent des données.
fn lire_hex(texte: &str) -> Result<(Vec<u8>, Vec<usize>), String> {
    let illisible = || "Ce .hex est abîmé : rien n’a été écrit.".to_string();
    let (mut image, mut pages) = (vec![0xFF; APPLI], vec![false; APPLI / PAGE]);
    let (mut base, mut mmu) = (0, false);
    for ligne in texte.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if let Some(entete) = ligne.strip_prefix(';') {
            // « ; device = mm-control » : l’en-tête par lequel PrusaSlicer reconnaît un firmware de MMU
            mmu |= entete.split('=').map(str::trim).eq(["device", "mm-control"]);
            continue;
        }
        let h = ligne.strip_prefix(':').filter(|h| h.len() >= 10 && h.len() % 2 == 0 && h.is_ascii()).ok_or_else(illisible)?;
        let o = (0..h.len()).step_by(2).map(|i| u8::from_str_radix(&h[i..i + 2], 16).ok()).collect::<Option<Vec<u8>>>().ok_or_else(illisible)?;
        let n = o[0] as usize;
        if o.len() != n + 5 || o.iter().fold(0u8, |s, x| s.wrapping_add(*x)) != 0 {
            return Err(illisible());
        }
        let donnees = &o[4..4 + n];
        match (o[3], n) {
            (0, 0) => {}
            (0, _) => {
                let debut = base + u16::from_be_bytes([o[1], o[2]]) as usize;
                if debut + n > APPLI {
                    return Err("Ce firmware est trop gros pour le MMU3 : il déborderait sur son chargeur. Rien n’a été écrit.".into());
                }
                image[debut..debut + n].copy_from_slice(donnees);
                pages[debut / PAGE..=(debut + n - 1) / PAGE].fill(true);
            }
            (1, _) if !mmu => return Err("Ce .hex n’est pas un firmware de MMU : rien n’a été écrit.".into()),
            (1, _) if !pages[0] => return Err(illisible()), // pas de vecteurs à l’adresse 0
            (1, _) => return Ok((image, (0..pages.len()).filter(|&p| pages[p]).collect())),
            (2, 2) => base = (u16::from_be_bytes([donnees[0], donnees[1]]) as usize) << 4,
            (4, 2) => base = (u16::from_be_bytes([donnees[0], donnees[1]]) as usize) << 16,
            (3 | 5, _) => {}
            _ => return Err(illisible()),
        }
    }
    Err(illisible()) // pas d’enregistrement de fin : fichier tronqué
}

fn muet(e: std::io::Error) -> String {
    format!("Le MMU ne répond plus ({e}).")
}

fn lire<P: Read>(port: &mut P, n: usize) -> Result<Vec<u8>, String> {
    let mut v = vec![0; n];
    port.read_exact(&mut v).map_err(muet)?;
    Ok(v)
}

fn envoyer<P: Write>(port: &mut P, octets: &[u8]) -> Result<(), String> {
    port.write_all(octets).and_then(|_| port.flush()).map_err(muet)
}

/// Une commande AVR109 à laquelle le chargeur répond par un retour chariot.
fn commande<P: Read + Write>(port: &mut P, octets: &[u8]) -> Result<(), String> {
    envoyer(port, octets)?;
    match lire(port, 1)?[..] {
        [b'\r'] => Ok(()),
        _ => Err(format!("Le chargeur du MMU a refusé la commande « {} ».", octets[0] as char)),
    }
}

/// « A » : l’adresse en mots de 16 bits, poids fort d’abord.
fn adresse(octet: usize) -> [u8; 3] {
    let [h, l] = ((octet / 2) as u16).to_be_bytes();
    [b'A', h, l]
}

/// « B » : un bloc écrit en flash (Caterina efface la page puis l’écrit).
fn bloc(octets: &[u8]) -> Vec<u8> {
    let [h, l] = (octets.len() as u16).to_be_bytes();
    [&[b'B', h, l, b'F'][..], octets].concat()
}

/// « g » : un bloc lu en flash.
fn lecture(n: usize) -> [u8; 4] {
    let [h, l] = (n as u16).to_be_bytes();
    [b'g', h, l, b'F']
}

/// La présentation d’avrdude (butterfly_initialize), dans son ordre, puis la signature de l’ATmega32U4.
fn presenter<P: Read + Write>(port: &mut P) -> Result<(), String> {
    envoyer(port, b"\x1bS")?; // Échap réveille les chargeurs « butterfly », Caterina l’ignore ; S : son nom en 7 lettres
    lire(port, 7)?;
    envoyer(port, b"V")?;
    lire(port, 2)?;
    envoyer(port, b"v")?;
    if lire(port, 1)? != [b'?'] {
        lire(port, 1)?;
    }
    envoyer(port, b"p")?;
    lire(port, 1)?;
    envoyer(port, b"a")?;
    lire(port, 1)?;
    envoyer(port, b"b")?;
    let b = lire(port, 3)?;
    if b[0] != b'Y' || (u16::from_be_bytes([b[1], b[2]]) as usize) < PAGE {
        return Err("Le chargeur du MMU n’écrit pas par pages : rien n’a été écrit.".into());
    }
    envoyer(port, b"t")?;
    let mut appareil = None;
    loop {
        match lire(port, 1)?[0] {
            0 => break,
            c => {
                appareil.get_or_insert(c);
            }
        }
    }
    commande(port, &[b'T', appareil.unwrap_or(0)])?;
    commande(port, b"P")?;
    envoyer(port, b"s")?;
    if lire(port, 3)? != [0x87, 0x95, 0x1E] {
        return Err("Ce n’est pas la puce d’un MMU3 (signature inattendue) : rien n’a été écrit.".into());
    }
    Ok(())
}

fn ecrire_page<P: Read + Write>(port: &mut P, page: usize, octets: &[u8]) -> Result<(), String> {
    commande(port, &adresse(page * PAGE))?;
    commande(port, &bloc(octets))
}

fn relire_page<P: Read + Write>(port: &mut P, page: usize) -> Result<Vec<u8>, String> {
    commande(port, &adresse(page * PAGE))?;
    envoyer(port, &lecture(PAGE))?;
    lire(port, PAGE)
}

/// Les pages écrites puis relues. La première est effacée d’abord et écrite en dernier : tant qu’elle est vide, Caterina
/// ne lance pas de firmware et reste dans son chargeur sans délai, si bien qu’une installation interrompue se relance.
fn flasher<P: Read + Write>(port: &mut P, image: &[u8], pages: &[usize], mut avance: impl FnMut(usize, usize)) -> Result<(), String> {
    presenter(port)?;
    ecrire_page(port, 0, &[0xFF; PAGE])?;
    let autres: Vec<usize> = pages.iter().copied().filter(|&p| p != 0).collect();
    let (total, mut fait) = (2 * (autres.len() + 1), 0);
    for passe in [&autres[..], &[0]] {
        for &p in passe {
            ecrire_page(port, p, &image[p * PAGE..][..PAGE])?;
            fait += 1;
            avance(fait, total);
        }
        for &p in passe {
            if relire_page(port, p)? != image[p * PAGE..][..PAGE] {
                return Err(format!("La relecture ne correspond pas à l’adresse 0x{:04X}.", p * PAGE));
            }
            fait += 1;
            avance(fait, total);
        }
    }
    commande(port, b"L")?;
    commande(port, b"E") // le chargeur lance le nouveau firmware une demi-seconde plus tard
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::io;

    /// Un enregistrement Intel HEX avec sa somme de contrôle.
    fn enr(adr: u16, genre: u8, donnees: &[u8]) -> String {
        let o = [&[donnees.len() as u8, (adr >> 8) as u8, adr as u8, genre][..], donnees].concat();
        let somme = o.iter().fold(0u8, |s, x| s.wrapping_add(*x)).wrapping_neg();
        format!(":{}{:02X}\r\n", o.iter().map(|x| format!("{x:02X}")).collect::<String>(), somme)
    }

    fn hex(enregistrements: &[String]) -> String {
        format!("; device = mm-control\r\n\r\n{}{}", enregistrements.concat(), enr(0, 1, &[]))
    }

    #[test]
    fn lit_le_hex_du_mmu() {
        assert_eq!(enr(0, 1, &[]), ":00000001FF\r\n");
        let (image, pages) = lire_hex(&hex(&[enr(0, 0, &[0x0C, 0x94]), enr(0x0130, 0, &[1, 2, 3])])).unwrap();
        assert_eq!((&image[..3], &image[0x130..0x134]), (&[0x0C, 0x94, 0xFF][..], &[1, 2, 3, 0xFF][..]));
        assert_eq!(pages, [0, 2]);
        // adresse de base étendue (type 02) : 0x0010 << 4 = 0x100, page 2
        assert_eq!(lire_hex(&hex(&[enr(0, 0, &[1]), enr(0, 2, &[0, 0x10]), enr(0, 0, &[7])])).unwrap().1, [0, 2]);
    }

    #[test]
    fn refuse_les_hex_douteux() {
        let bon = hex(&[enr(0, 0, &[1, 2])]);
        assert!(lire_hex(&bon).is_ok());
        assert!(lire_hex(&bon.replace("; device = mm-control", "; device = mk3")).is_err(), "pas un MMU");
        assert!(lire_hex(&bon.replace(":0200000001", ":0200000002")).is_err(), "somme fausse");
        assert!(lire_hex(&bon.replace(":00000001FF", "")).is_err(), "tronqué");
        assert!(lire_hex(&hex(&[enr(0, 0, &[1]), enr(0x6FFF, 0, &[1, 2])])).is_err(), "déborde sur le chargeur");
        assert!(lire_hex(&hex(&[enr(0, 0, &[1]), enr(0x6FFE, 0, &[1, 2])])).is_ok(), "dernier mot du firmware");
        assert!(lire_hex(&hex(&[enr(0x80, 0, &[1])])).is_err(), "sans vecteurs en 0");
    }

    #[test]
    fn trames_avr109() {
        assert_eq!(adresse(0x6F80), [b'A', 0x37, 0xC0]);
        assert_eq!(bloc(&[1, 2]), [b'B', 0, 2, b'F', 1, 2]);
        assert_eq!(lecture(PAGE), [b'g', 0, 0x80, b'F']);
    }

    /// Un chargeur Caterina simulé : il répond comme Caterina.c et garde l’ordre des pages écrites.
    struct Caterina {
        flash: Vec<u8>,
        recu: Vec<u8>,
        reponse: VecDeque<u8>,
        adresse: usize,
        ecrites: Vec<(usize, u8)>,
        panne: Option<usize>,
    }

    impl Caterina {
        fn new() -> Self {
            Caterina { flash: vec![0x55; APPLI], recu: vec![], reponse: VecDeque::new(), adresse: 0, ecrites: vec![], panne: None }
        }
    }

    impl Write for Caterina {
        fn write(&mut self, o: &[u8]) -> io::Result<usize> {
            self.recu.extend_from_slice(o);
            loop {
                let r = self.recu.clone();
                let taille = |i: usize| (r[i] as usize) << 8 | r[i + 1] as usize;
                let (lu, rep): (usize, Vec<u8>) = match r[..] {
                    [] => break,
                    [0x1b, ..] => (1, vec![]),
                    [b'S', ..] => (1, b"CATERIN".to_vec()),
                    [b'V', ..] => (1, b"10".to_vec()),
                    [b'v', ..] => (1, b"?".to_vec()),
                    [b'p', ..] => (1, b"S".to_vec()),
                    [b'a', ..] => (1, b"Y".to_vec()),
                    [b'b', ..] => (1, vec![b'Y', 0, 128]),
                    [b't', ..] => (1, vec![0x44, 0]),
                    [b'T', _, ..] | [b'P' | b'L' | b'E', ..] => (if r[0] == b'T' { 2 } else { 1 }, b"\r".to_vec()),
                    [b's', ..] => (1, vec![0x87, 0x95, 0x1E]),
                    [b'A', h, l, ..] => {
                        self.adresse = (h as usize) << 9 | (l as usize) << 1;
                        (3, b"\r".to_vec())
                    }
                    [b'B', _, _, b'F', ..] if r.len() >= 4 + taille(1) => {
                        let mut page = vec![0xFF; PAGE];
                        page[..taille(1)].copy_from_slice(&r[4..4 + taille(1)]);
                        if self.panne == Some(self.adresse / PAGE) {
                            page[5] ^= 1;
                        }
                        self.ecrites.push((self.adresse / PAGE, page[0]));
                        self.flash[self.adresse..][..PAGE].copy_from_slice(&page);
                        (4 + taille(1), b"\r".to_vec())
                    }
                    [b'g', _, _, b'F', ..] => {
                        let n = taille(1);
                        self.adresse += n;
                        (4, self.flash[self.adresse - n..self.adresse].to_vec())
                    }
                    _ => break, // commande pas encore complète
                };
                self.recu.drain(..lu);
                self.reponse.extend(rep);
            }
            Ok(o.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl Read for Caterina {
        fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
            let n = b.len().min(self.reponse.len());
            if n == 0 {
                return Err(io::ErrorKind::TimedOut.into());
            }
            for x in &mut b[..n] {
                *x = self.reponse.pop_front().unwrap();
            }
            Ok(n)
        }
    }

    fn image() -> (Vec<u8>, Vec<usize>) {
        lire_hex(&hex(&[enr(0, 0, &[0x0C, 0x94, 0xF7, 0x04]), enr(0x200, 0, &[9; 16]), enr(0x6F80, 0, &[3; 4])])).unwrap()
    }

    #[test]
    fn flashe_et_relit_page_zero_en_dernier() {
        let ((image, pages), mut mmu, mut etapes) = (image(), Caterina::new(), vec![]);
        flasher(&mut mmu, &image, &pages, |f, t| etapes.push((f, t))).unwrap();
        for &p in &pages {
            assert_eq!(mmu.flash[p * PAGE..][..PAGE], image[p * PAGE..][..PAGE]);
        }
        // page 0 effacée d’abord, écrite en dernier ; les pages sans données ne sont pas touchées
        assert_eq!(mmu.ecrites, [(0, 0xFF), (4, 9), (0xDF, 3), (0, 0x0C)]);
        assert_eq!(mmu.flash[PAGE], 0x55);
        assert_eq!(etapes.last(), Some(&(6, 6)));
        assert!(mmu.recu.is_empty() && mmu.reponse.is_empty());
    }

    #[test]
    fn une_relecture_fausse_laisse_le_chargeur_joignable() {
        let ((image, pages), mut mmu) = (image(), Caterina::new());
        mmu.panne = Some(4);
        let e = flasher(&mut mmu, &image, &pages, |_, _| {}).unwrap_err();
        assert!(e.contains("0x0200"), "{e}");
        assert_eq!(mmu.flash[..2], [0xFF, 0xFF], "page 0 vide : Caterina reste dans son chargeur");
    }

    #[test]
    fn liste_les_versions_du_mmu() {
        let rel = |tag: &str, nom: &str, digest: Option<&str>| {
            json!({ "tag_name": tag, "prerelease": false, "assets": [{ "name": nom, "digest": digest, "size": 1, "browser_download_url": "u" }] })
        };
        let lues = [
            ("OneDrop3D", json!([rel("od3d-3.0.4.940", "MMU2S_MMU3_FW3.0.4+940.hex", Some("sha256:ab")), rel("od3d-3.0.4.1000", "MMU2S_MMU3_FW3.0.4+1000.hex", Some("sha256:cd"))])),
            ("Prusa", json!([rel("v3.0.3", "MMU2S_MMU3_FW3.0.3+896.hex", None), rel("v3.0.4", "MMU3_FW3.0.4+936.hex", Some("sha256:ef")), rel("v3.0.4", "notes.txt", Some("sha256:00"))])),
        ];
        let v = crate::firmware::retenir(&lues, mmu3);
        let tags: Vec<_> = v.iter().map(|v| (v["source"].as_str().unwrap(), v["version"].as_str().unwrap())).collect();
        assert_eq!(tags, [("OneDrop3D", "od3d-3.0.4.1000"), ("OneDrop3D", "od3d-3.0.4.940"), ("Prusa", "v3.0.4")]);
        assert_eq!(v[2]["sha256"], "ef");
    }
}
