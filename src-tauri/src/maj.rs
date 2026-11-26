/* Mises à jour de l’application : le manifeste publié dans le dépôt OneDrop3D/mises-a-jour, au même format que celui
 * de Multibook. Pas encore publié (404) ou pas de connexion : pas de bandeau, rien à signaler. */
use crate::{client, telecharger};
use serde_json::{json, Value};
use std::fs;
use std::time::Duration;
use tauri::{AppHandle, Manager};

const MANIFESTE: &str = "https://raw.githubusercontent.com/OneDrop3D/mises-a-jour/main/printforge.json";

/// La clé de ce système dans « files » du manifeste.
fn plateforme() -> Option<&'static str> {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some("macos-arm64")
    } else if cfg!(all(windows, target_arch = "x86_64")) {
        Some("windows-x64")
    } else {
        None
    }
}

/// 1.18.9 et 1.18.9.0 sont la même version ; un morceau non numérique compte pour 0.
fn cle_version(v: &str) -> Vec<u64> {
    let mut cle: Vec<u64> = v.split(['.', '-']).map(|x| x.parse().unwrap_or(0)).collect();
    while cle.last() == Some(&0) {
        cle.pop();
    }
    cle
}

/// Le manifeste, ou None s’il n’est pas encore publié.
fn manifeste() -> Result<Option<Value>, String> {
    let mut r = client(Duration::from_secs(15)).get(MANIFESTE).call().map_err(|e| tr!("Pas de connexion à GitHub ({e}).", "No connection to GitHub ({e})."))?;
    match r.status().as_u16() {
        404 => Ok(None),
        200 => {
            let texte = r.body_mut().read_to_string().map_err(|e| e.to_string())?;
            serde_json::from_str(&texte).map(Some).map_err(|_| tr!("Manifeste de mise à jour illisible.", "Unreadable update manifest."))
        }
        code => Err(tr!("GitHub a répondu {code}.", "GitHub answered {code}.")),
    }
}

/// La version publiée si elle est plus récente que celle-ci et existe pour ce système ; sinon null.
#[tauri::command(async)]
pub fn maj_verifier(app: AppHandle) -> Result<Value, String> {
    let Some(m) = manifeste()? else { return Ok(Value::Null) };
    let nouvelle = m["version"].as_str().unwrap_or("");
    let pour_ce_systeme = plateforme().is_some_and(|p| m["files"].get(p).is_some());
    if !pour_ce_systeme || cle_version(nouvelle) <= cle_version(&app.package_info().version.to_string()) {
        return Ok(Value::Null);
    }
    Ok(json!({ "version": nouvelle, "notes": m["notes"], "page": m["page"] }))
}

/// Télécharge l’installateur dans Téléchargements, vérifie sa taille et son empreinte, puis l’ouvre.
#[tauri::command(async)]
pub fn maj_installer(app: AppHandle) -> Result<String, String> {
    let m = manifeste()?.ok_or_else(|| tr!("Aucune mise à jour n’est publiée.", "No update is published."))?;
    let f = &m["files"][plateforme().ok_or_else(|| tr!("Pas de mise à jour pour ce système.", "No update for this system."))?];
    let nom = f["name"].as_str().unwrap_or("");
    if nom.is_empty() || nom.starts_with('.') || nom.contains(['/', '\\']) {
        return Err(tr!("Manifeste de mise à jour invalide.", "Invalid update manifest."));
    }
    let octets = telecharger(f["url"].as_str().unwrap_or(""), f["size"].as_u64().unwrap_or(0), f["sha256"].as_str().unwrap_or(""))?;
    let chemin = app.path().download_dir().map_err(|e| e.to_string())?.join(nom);
    fs::write(&chemin, &octets).map_err(|e| tr!("Enregistrement impossible dans Téléchargements : {e}", "Could not save to Downloads: {e}"))?;
    tauri_plugin_opener::open_path(&chemin, None::<&str>)
        .map_err(|e| tr!("L’installateur est dans Téléchargements, mais n’a pas pu être ouvert : {e}", "The installer is in Downloads, but could not be opened: {e}"))?;
    Ok(chemin.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::cle_version as v;

    #[test]
    fn comparaison_des_versions() {
        assert!(v("0.1.1") > v("0.1.0"));
        assert!(v("0.10.0") > v("0.9.9"));
        assert!(v("1.0") > v("0.99.99"));
        assert_eq!(v("1.18.9"), v("1.18.9.0"));
        assert!(v("0.1.0") <= v("0.1.0"));
    }
}
