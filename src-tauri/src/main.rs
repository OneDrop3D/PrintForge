#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
/* OneDrop PrintForge : le compagnon des CORE One qui ont le firmware OneDrop3D. Un module par partie : l’imprimante
 * et l’envoi de fichiers (PrusaLink), la musique, le firmware (clé USB ou réseau), celui du MMU3 (port USB du MMU),
 * les mises à jour de l’application. */
mod firmware;
mod imprimante;
mod maj;
mod mmu;
mod musique;

use sha2::{Digest, Sha256};
use std::time::Duration;

/// Un client HTTP dont les réponses 4xx et 5xx restent des réponses : chaque appel traduit son code en français.
fn client(delai: Duration) -> ureq::Agent {
    ureq::Agent::config_builder().timeout_global(Some(delai)).http_status_as_error(false).build().into()
}

/// Un fichier publié (firmware, installateur), rendu seulement si sa taille et son empreinte sont les bonnes.
fn telecharger(url: &str, taille: u64, sha256: &str) -> Result<Vec<u8>, String> {
    if !url.starts_with("https://") {
        return Err(format!("Adresse de téléchargement refusée : {url}"));
    }
    let mut r = client(Duration::from_secs(600))
        .get(url)
        .call()
        .map_err(|e| format!("Téléchargement impossible : vérifiez la connexion à Internet ({e})."))?;
    if r.status() != 200 {
        return Err(format!("Téléchargement impossible (code {}).", r.status().as_u16()));
    }
    // un octet de plus que prévu suffit à voir qu’un fichier est trop long
    let octets = r
        .body_mut()
        .with_config()
        .limit(taille + 1)
        .read_to_vec()
        .map_err(|e| format!("Téléchargement interrompu ({e}) : réessayez."))?;
    if octets.len() as u64 != taille || !format!("{:x}", Sha256::digest(&octets)).eq_ignore_ascii_case(sha256) {
        return Err("Le fichier est arrivé abîmé (taille ou empreinte différente) : réessayez.".into());
    }
    Ok(octets)
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            imprimante::imprimante_config,
            imprimante::imprimante_enregistrer,
            imprimante::imprimante_etat,
            imprimante::imprimantes_chercher,
            imprimante::envoyer_fichier,
            imprimante::imprimer,
            musique::musique_envoyer,
            musique::musique_cle,
            firmware::firmware_versions,
            firmware::firmware_cles,
            firmware::firmware_copier,
            firmware::firmware_reseau,
            mmu::mmu_versions,
            mmu::mmu_detecter,
            mmu::mmu_flasher,
            maj::maj_verifier,
            maj::maj_installer
        ])
        .run(tauri::generate_context!())
        .expect("impossible de démarrer OneDrop PrintForge");
}
