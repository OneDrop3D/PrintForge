/* Le firmware de la CORE One : les versions publiées sur GitHub, la clé USB qui sert au flash, et l’installation par
 * le réseau quand le firmware de l’imprimante la permettra. Les règles de la clé sont celles, éprouvées, de l’onglet
 * Firmware de OneDrop3D. */
use crate::imprimante::{refus, Imprimante, USB};
use crate::{client, telecharger};
use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::path::Path;
use std::time::{Duration, Instant};
use tauri::AppHandle;

const DEPOTS: [(&str, &str); 2] = [("OneDrop3D", "OneDrop3D/Prusa-Firmware-Buddy"), ("Prusa", "prusa3d/Prusa-Firmware-Buddy")];
/// Le nom du firmware envoyé par le réseau sur la clé de l’imprimante.
const BBF_RESEAU: &str = "OD3DFW.BBF";

/// Le .bbf de la CORE One (One+, One+ Gen2) d’une release, sans ceux de la CORE One L ni de l’INDX.
fn coreone(source: &str, nom: &str) -> bool {
    if source == "OneDrop3D" {
        return nom.contains("-OD3D") && nom.ends_with(".bbf"); // l’officiel joint à nos releases est listé avec Prusa
    }
    nom.starts_with("COREONE_") && !nom.starts_with("COREONE_L") && !nom.contains("INDX") && nom.ends_with(".bbf")
}

/// Les nôtres d’abord, puis les officiels. Seuls les fichiers dont GitHub donne l’empreinte sont proposés.
#[tauri::command(async)]
pub fn firmware_versions() -> Result<Vec<Value>, String> {
    versions(&DEPOTS, coreone)
}

/// Les fichiers que `garder(source, nom)` retient dans les releases de ces dépôts (le firmware de l’imprimante ou du MMU).
pub(crate) fn versions(depots: &[(&str, &str)], garder: fn(&str, &str) -> bool) -> Result<Vec<Value>, String> {
    let mut lues = vec![];
    for (source, depot) in depots {
        let mut r = client(Duration::from_secs(30))
            .get(format!("https://api.github.com/repos/{depot}/releases?per_page=50"))
            .call()
            .map_err(|e| format!("GitHub ne répond pas : vérifiez la connexion à Internet ({e})."))?;
        if r.status() != 200 {
            return Err(format!("GitHub refuse la liste des versions (code {}) : réessayez plus tard.", r.status().as_u16()));
        }
        let releases: Value = serde_json::from_str(&r.body_mut().read_to_string().map_err(|e| e.to_string())?)
            .map_err(|_| "Liste des versions illisible.".to_string())?;
        lues.push((*source, releases));
    }
    Ok(retenir(&lues, garder))
}

/// Les fichiers retenus des releases lues, chaque source dans l’ordre des dépôts.
pub(crate) fn retenir(lues: &[(&str, Value)], garder: fn(&str, &str) -> bool) -> Vec<Value> {
    let mut versions = vec![];
    for (source, releases) in lues {
        for rel in releases.as_array().into_iter().flatten() {
            for a in rel["assets"].as_array().into_iter().flatten() {
                let (nom, empreinte) = (a["name"].as_str().unwrap_or(""), a["digest"].as_str().unwrap_or(""));
                if garder(source, nom) && empreinte.starts_with("sha256:") {
                    versions.push(json!({
                        "source": source, "version": rel["tag_name"], "beta": rel["prerelease"], "nom": nom,
                        "url": a["browser_download_url"], "taille": a["size"], "sha256": &empreinte[7..]
                    }));
                }
            }
        }
    }
    // GitHub ne range pas les releases dans l'ordre des versions (od3d-8.0.2.10 arrivait après la .4) :
    // OneDrop3D d'abord, puis la plus récente d'abord dans chaque source.
    let rang = |v: &Value| lues.iter().position(|(s, _)| v["source"] == *s).unwrap_or(usize::MAX);
    versions.sort_by(|a, b| rang(a).cmp(&rang(b)).then_with(|| numeros(b).cmp(&numeros(a))));
    versions
}

/// « od3d-8.0.2.10 » → ([8, 0, 2, 10], finale) ; « v6.4.0-RC2 » → ([6, 4, 0], bêta) : comparées en nombres, la finale avant ses bêtas.
fn numeros(v: &Value) -> (Vec<u64>, bool) {
    let tag = v["version"].as_str().unwrap_or("");
    let base = tag.trim_start_matches("od3d-").trim_start_matches('v').split('-').next().unwrap_or("");
    (base.split('.').filter_map(|n| n.parse().ok()).collect(), v["beta"] != true)
}

#[cfg(test)]
mod tests {
    #[test]
    fn ecrit_sur_la_cle_sans_laisser_de_part() {
        let dossier = std::env::temp_dir().join(format!("printforge-cle-{}", std::process::id()));
        std::fs::create_dir_all(&dossier).unwrap();
        super::ecrire_sur_cle(dossier.to_str().unwrap(), "musique-essai.gcode", b"G28\n").unwrap();
        assert_eq!(std::fs::read(dossier.join("musique-essai.gcode")).unwrap(), b"G28\n");
        assert!(!dossier.join("od3d-telechargement.part").exists());
        std::fs::remove_dir_all(&dossier).unwrap();
    }

    #[test]
    fn la_plus_recente_d_abord() {
        let v = |t: &str, beta: bool| serde_json::json!({ "version": t, "beta": beta });
        assert!(super::numeros(&v("od3d-8.0.2.10", false)) > super::numeros(&v("od3d-8.0.2.9", false)));
        assert!(super::numeros(&v("v8.0.3-beta", true)) > super::numeros(&v("v8.0.2", false)));
        assert!(super::numeros(&v("v6.4.0", false)) > super::numeros(&v("v6.4.0-RC2", true)));
        assert_eq!(super::numeros(&v("od3d-8.0.2", false)).0, vec![8, 0, 2]);
    }
}

/// Le fichier d’une version reçue de la page (url, nom, taille, sha256), seulement s’il vient d’une release du firmware.
pub(crate) fn fichier(depots: &[(&str, &str)], garder: fn(&str, &str) -> bool, v: &Value) -> Result<(String, String, u64, String), String> {
    let (source, url, nom) = (v["source"].as_str().unwrap_or(""), v["url"].as_str().unwrap_or(""), v["nom"].as_str().unwrap_or(""));
    let depot = depots.iter().find(|(s, _)| *s == source).map(|(_, d)| format!("https://github.com/{d}/releases/download/"));
    if !(depot.is_some_and(|d| url.starts_with(&d)) && !nom.contains(['/', '\\']) && garder(source, nom)) {
        return Err("Ce fichier ne fait pas partie des versions du firmware.".into());
    }
    Ok((url.into(), nom.into(), v["taille"].as_u64().unwrap_or(0), v["sha256"].as_str().unwrap_or("").into()))
}

#[cfg(target_os = "macos")]
fn diskutil(cible: &str) -> plist::Dictionary {
    let sortie = std::process::Command::new("diskutil").args(["info", "-plist", cible]).output().ok();
    sortie
        .filter(|s| s.status.success())
        .and_then(|s| plist::Value::from_reader_xml(&s.stdout[..]).ok())
        .and_then(|v| v.into_dictionary())
        .unwrap_or_default()
}

/// Les clés USB montées, avec ce qui empêcherait l’imprimante de les lire (FAT32 et schéma MBR, d’après Prusa).
#[cfg(target_os = "macos")]
fn cles() -> Vec<Value> {
    let texte = |d: &plist::Dictionary, k: &str| d.get(k).and_then(|v| v.as_string()).unwrap_or("").to_string();
    let mut noms: Vec<String> =
        fs::read_dir("/Volumes").into_iter().flatten().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    noms.sort();
    noms.into_iter()
        .filter_map(|nom| {
            let chemin = format!("/Volumes/{nom}");
            let info = diskutil(&chemin);
            if texte(&info, "BusProtocol") != "USB" || texte(&info, "MountPoint") != chemin {
                return None;
            }
            let probleme = if texte(&info, "FilesystemType") != "msdos" {
                let format = Some(texte(&info, "FilesystemUserVisibleName")).filter(|f| !f.is_empty());
                format!("elle est en {}, et l’imprimante ne lit que le FAT32", format.as_deref().unwrap_or("format inconnu"))
            } else if texte(&diskutil(&texte(&info, "ParentWholeDisk")), "Content") != "FDisk_partition_scheme" {
                "son schéma de partition n’est pas MBR".into()
            } else {
                String::new()
            };
            Some(json!({ "chemin": chemin, "nom": nom, "probleme": probleme }))
        })
        .collect()
}

/// Windows : les lecteurs amovibles et leur format.
// shortcut: le schéma MBR n’est pas vérifié sous Windows (l’imprimante le dira) ; à ajouter si des clés GPT posent problème.
#[cfg(windows)]
fn cles() -> Vec<Value> {
    sysinfo::Disks::new_with_refreshed_list()
        .iter()
        .filter(|d| d.is_removable())
        .map(|d| {
            let chemin = d.mount_point().to_string_lossy().into_owned();
            let (format, etiquette) = (d.file_system().to_string_lossy(), d.name().to_string_lossy());
            let probleme = if format.eq_ignore_ascii_case("FAT32") {
                String::new()
            } else {
                format!("elle est en {format}, et l’imprimante ne lit que le FAT32")
            };
            let nom = if etiquette.is_empty() { chemin.clone() } else { format!("{etiquette} ({})", chemin.trim_end_matches('\\')) };
            json!({ "chemin": chemin, "nom": nom, "probleme": probleme })
        })
        .collect()
}

// shortcut: pas de détection des clés sous Linux ; à écrire si l’application y est publiée.
#[cfg(not(any(target_os = "macos", windows)))]
fn cles() -> Vec<Value> {
    vec![]
}

/// macOS : la clé éjectée, ou false si elle n’a pas pu l’être. Ailleurs : None, l’utilisateur l’éjecte lui-même.
pub(crate) fn ejecter(cle: &str) -> Option<bool> {
    #[cfg(target_os = "macos")]
    return Some(std::process::Command::new("diskutil").args(["eject", cle]).output().is_ok_and(|s| s.status.success()));
    #[cfg(not(target_os = "macos"))]
    {
        let _ = cle;
        None
    }
}

#[tauri::command(async)]
pub fn firmware_cles() -> Vec<Value> {
    cles()
}

/// La clé est toujours branchée, et l’imprimante pourra la lire.
pub(crate) fn cle_lisible(cle: &str) -> Result<(), String> {
    if cles().iter().any(|c| c["chemin"] == cle && c["probleme"] == "") {
        return Ok(());
    }
    Err("Cette clé n’est plus branchée, ou l’imprimante ne pourrait pas la lire.".into())
}

/// Le fichier à la racine de la clé, jamais à moitié écrit sous les yeux de l’imprimante : un .part, renommé une fois complet.
pub(crate) fn ecrire_sur_cle(cle: &str, nom: &str, octets: &[u8]) -> Result<(), String> {
    let dossier = Path::new(cle);
    let partiel = dossier.join("od3d-telechargement.part");
    let ecrire = || -> std::io::Result<()> {
        let mut f = fs::File::create(&partiel)?;
        f.write_all(octets)?;
        f.sync_all()?;
        fs::rename(&partiel, dossier.join(nom))
    };
    ecrire().map_err(|e| {
        let _ = fs::remove_file(&partiel);
        format!("Écriture sur la clé impossible : {e}")
    })
}

/// Le .bbf vérifié, seul à la racine de la clé (règle de Prusa), puis la clé éjectée.
#[tauri::command(async)]
pub fn firmware_copier(cle: String, version: Value) -> Result<Value, String> {
    cle_lisible(&cle)?;
    let (url, nom, taille, sha256) = fichier(&DEPOTS, coreone, &version)?;
    let octets = telecharger(&url, taille, &sha256)?;
    ecrire_sur_cle(&cle, &nom, &octets)?;
    let dossier = Path::new(&cle);
    for entree in fs::read_dir(dossier).into_iter().flatten().flatten() {
        let n = entree.file_name().to_string_lossy().into_owned();
        // les anciens firmwares, et les « ._ » que macOS crée à côté sur une clé FAT (le nôtre compris)
        if n.to_lowercase().ends_with(".bbf") && n != nom {
            let _ = fs::remove_file(entree.path()); // macOS efface parfois le « ._ » avec son fichier
        }
    }
    Ok(json!({ "nom": nom, "ejectee": ejecter(&cle) }))
}

/// Installation par le réseau : l’imprimante au repos reçoit le .bbf vérifié, puis redémarre pour le flasher.
#[tauri::command(async)]
pub fn firmware_reseau(app: AppHandle, version: Value) -> Result<Value, String> {
    let imprimante = Imprimante::reliee(&app)?;
    let (url, _, taille, sha256) = fichier(&DEPOTS, coreone, &version)?;
    let etat = imprimante.lire("/api/v1/status")?["printer"]["state"].as_str().unwrap_or("").to_string();
    if !["IDLE", "READY", "FINISHED", "STOPPED"].contains(&etat.as_str()) {
        return Err(format!("Installation refusée : l’imprimante doit être au repos, sans impression en cours (état : {etat})."));
    }
    let avant = imprimante.lire("/api/version")?["firmware"].as_str().unwrap_or("").to_string();
    let octets = telecharger(&url, taille, &sha256)?;
    let chemin = format!("{USB}{BBF_RESEAU}");
    match imprimante.deposer(BBF_RESEAU, &octets[..], taille, false)? {
        (201, _) => {}
        (415, _) => {
            return Err("Le firmware de votre imprimante ne permet pas encore l’installation par le réseau : utilisez une clé USB (ci-dessus).".into())
        }
        (code, corps) => return Err(refus(code, &corps)),
    }
    // l’imprimante redémarre pour flasher : la connexion peut tomber avant toute réponse
    if let Ok((code, _)) = imprimante.requete("POST", &chemin, &[], &b""[..], Duration::from_secs(10)) {
        if code >= 300 {
            let _ = imprimante.requete("DELETE", &chemin, &[], (), Duration::from_secs(5));
            return Err(format!("L’imprimante n’a pas lancé l’installation (code {code})."));
        }
    }
    let fin = Instant::now() + Duration::from_secs(180);
    let mut apres = None;
    while apres.is_none() && Instant::now() < fin {
        std::thread::sleep(Duration::from_secs(5));
        let firmware = imprimante.lire("/api/version").ok().and_then(|v| v["firmware"].as_str().map(String::from));
        apres = firmware.filter(|f| !f.is_empty() && *f != avant);
    }
    // retiré de la clé de l’imprimante, qui sinon le reproposerait à chaque démarrage
    let _ = imprimante.requete("DELETE", &chemin, &[], (), Duration::from_secs(5));
    match apres {
        Some(f) => Ok(json!({ "firmware": f })),
        None => Err("Pas de nouvelle version au bout de 3 minutes : regardez l’écran de l’imprimante. Pour une version identique ou plus ancienne, passez par la clé USB (molette au logo).".into()),
    }
}
