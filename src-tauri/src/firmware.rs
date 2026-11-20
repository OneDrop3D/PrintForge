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
    let mut versions = vec![];
    for (source, depot) in DEPOTS {
        let mut r = client(Duration::from_secs(30))
            .get(format!("https://api.github.com/repos/{depot}/releases?per_page=50"))
            .call()
            .map_err(|e| format!("GitHub ne répond pas : vérifiez la connexion à Internet ({e})."))?;
        if r.status() != 200 {
            return Err(format!("GitHub refuse la liste des versions (code {}) : réessayez plus tard.", r.status().as_u16()));
        }
        let releases: Value = serde_json::from_str(&r.body_mut().read_to_string().map_err(|e| e.to_string())?)
            .map_err(|_| "Liste des versions illisible.".to_string())?;
        for rel in releases.as_array().into_iter().flatten() {
            for a in rel["assets"].as_array().into_iter().flatten() {
                let (nom, empreinte) = (a["name"].as_str().unwrap_or(""), a["digest"].as_str().unwrap_or(""));
                if coreone(source, nom) && empreinte.starts_with("sha256:") {
                    versions.push(json!({
                        "source": source, "version": rel["tag_name"], "beta": rel["prerelease"], "nom": nom,
                        "url": a["browser_download_url"], "taille": a["size"], "sha256": &empreinte[7..]
                    }));
                }
            }
        }
    }
    Ok(versions)
}

/// Le fichier d’une version reçue de la page (url, nom, taille, sha256), seulement s’il vient d’une release du firmware.
fn fichier(v: &Value) -> Result<(String, String, u64, String), String> {
    let (source, url, nom) = (v["source"].as_str().unwrap_or(""), v["url"].as_str().unwrap_or(""), v["nom"].as_str().unwrap_or(""));
    let depot = DEPOTS.iter().find(|(s, _)| *s == source).map(|(_, d)| format!("https://github.com/{d}/releases/download/"));
    if !(depot.is_some_and(|d| url.starts_with(&d)) && !nom.contains(['/', '\\']) && coreone(source, nom)) {
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
fn ejecter(cle: &str) -> Option<bool> {
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

/// Le .bbf vérifié, seul à la racine de la clé (règle de Prusa), puis la clé éjectée.
#[tauri::command(async)]
pub fn firmware_copier(cle: String, version: Value) -> Result<Value, String> {
    if !cles().iter().any(|c| c["chemin"] == cle.as_str() && c["probleme"] == "") {
        return Err("Cette clé n’est plus branchée, ou l’imprimante ne pourrait pas la lire.".into());
    }
    let (url, nom, taille, sha256) = fichier(&version)?;
    let octets = telecharger(&url, taille, &sha256)?;
    let dossier = Path::new(&cle);
    // jamais un .bbf à moitié écrit sous les yeux de l’imprimante : un .part, renommé une fois complet
    let partiel = dossier.join("od3d-telechargement.part");
    let ecrire = || -> std::io::Result<()> {
        let mut f = fs::File::create(&partiel)?;
        f.write_all(&octets)?;
        f.sync_all()?;
        fs::rename(&partiel, dossier.join(&nom))
    };
    if let Err(e) = ecrire() {
        let _ = fs::remove_file(&partiel);
        return Err(format!("Écriture sur la clé impossible : {e}"));
    }
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
    let (url, _, taille, sha256) = fichier(&version)?;
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
