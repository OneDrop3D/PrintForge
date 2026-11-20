/* L’imprimante par PrusaLink : son adresse et son mot de passe, la recherche sur le réseau, son état et l’envoi de
 * fichiers. Tous les appels passent par ici et non par la page : l’imprimante refuserait une autre origine (CORS). */
use crate::client;
use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};

const COURT: Duration = Duration::from_secs(5);
/// La racine de la clé USB de l’imprimante, vue par PrusaLink.
pub const USB: &str = "/api/v1/files/usb/";

pub struct Imprimante {
    adresse: String,
    cle: String,
}

impl Imprimante {
    /// L’imprimante enregistrée dans la configuration.
    pub fn reliee(app: &AppHandle) -> Result<Self, String> {
        let c = config(app);
        match (c["adresse"].as_str(), c["mot_de_passe"].as_str()) {
            (Some(a), Some(m)) if !a.is_empty() => Ok(Self { adresse: a.into(), cle: m.into() }),
            _ => Err("Reliez d’abord votre imprimante, dans l’onglet Imprimante.".into()),
        }
    }

    /// Une requête PrusaLink : le code et le corps de la réponse. Sans réponse, ou mot de passe refusé : un message clair.
    pub fn requete(
        &self,
        methode: &str,
        chemin: &str,
        entetes: &[(&str, &str)],
        corps: impl ureq::AsSendBody,
        delai: Duration,
    ) -> Result<(u16, String), String> {
        let mut r = ureq::http::Request::builder()
            .method(methode)
            .uri(format!("http://{}{chemin}", self.adresse))
            .header("X-Api-Key", &self.cle);
        for &(nom, valeur) in entetes {
            r = r.header(nom, valeur);
        }
        let r = r.body(corps).map_err(|e| e.to_string())?;
        let mut reponse = client(delai).run(r).map_err(|e| {
            format!(
                "L’imprimante ne répond pas à l’adresse {} : vérifiez qu’elle est allumée et sur le même réseau que cet ordinateur ({e}).",
                self.adresse
            )
        })?;
        match reponse.status().as_u16() {
            401 => Err("Mot de passe refusé : recopiez celui affiché sur l’imprimante, dans Réglages › Réseau › PrusaLink.".into()),
            code => Ok((code, reponse.body_mut().read_to_string().unwrap_or_default())),
        }
    }

    pub fn lire(&self, chemin: &str) -> Result<Value, String> {
        match self.requete("GET", chemin, &[], (), COURT)? {
            (200, corps) => serde_json::from_str(&corps)
                .map_err(|_| "Réponse illisible : est-ce bien une imprimante Prusa à cette adresse ?".into()),
            (code, _) => Err(format!("L’imprimante a répondu {code} à {chemin}.")),
        }
    }

    /// Dépose un fichier à la racine de la clé USB de l’imprimante, à la place de celui qui porterait le même nom.
    pub fn deposer(&self, nom: &str, corps: impl ureq::AsSendBody, taille: u64, imprimer: bool) -> Result<(u16, String), String> {
        let mut entetes = vec![("Overwrite", "?1")];
        if imprimer {
            entetes.push(("Print-After-Upload", "?1"));
        }
        // le Wi-Fi de l’imprimante peut descendre vers 10 Ko/s : un gros fichier prend plusieurs minutes
        self.requete("PUT", &format!("{USB}{nom}"), &entetes, corps, Duration::from_secs(60 + taille / 10_000))
    }
}

fn chemin_config(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app.path().app_config_dir().map_err(|e| e.to_string())?.join("imprimante.json"))
}

fn config(app: &AppHandle) -> Value {
    let texte = chemin_config(app).ok().and_then(|c| fs::read_to_string(c).ok());
    texte.and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_else(|| json!({}))
}

fn ecrire_config(app: &AppHandle, valeur: &Value) -> Result<(), String> {
    let chemin = chemin_config(app)?;
    let ecrire = || -> std::io::Result<()> {
        fs::create_dir_all(chemin.parent().unwrap_or(Path::new(".")))?;
        let mut f = fs::File::create(&chemin)?;
        #[cfg(unix)]
        {
            // le mot de passe de l’imprimante : lisible par ce seul compte
            use std::os::unix::fs::PermissionsExt;
            f.set_permissions(fs::Permissions::from_mode(0o600))?;
        }
        f.write_all(valeur.to_string().as_bytes())
    };
    ecrire().map_err(|e| format!("Configuration impossible à enregistrer : {e}"))
}

/// Ce que la page peut savoir de la configuration : jamais le mot de passe lui-même.
#[tauri::command]
pub fn imprimante_config(app: AppHandle) -> Value {
    let c = config(&app);
    json!({
        "adresse": c["adresse"].as_str().unwrap_or(""),
        "motDePasse": c["mot_de_passe"].as_str().is_some_and(|m| !m.is_empty())
    })
}

/// Enregistre l’adresse et le mot de passe, puis vérifie que l’imprimante répond (sa version).
#[tauri::command(async)]
pub fn imprimante_enregistrer(app: AppHandle, adresse: String, mot_de_passe: String) -> Result<Value, String> {
    let saisie = adresse.trim().trim_start_matches("http://").trim_start_matches("https://");
    let adresse = saisie.split('/').next().unwrap_or("");
    if adresse.is_empty() || !adresse.chars().all(|c| c.is_ascii_alphanumeric() || ".-:[]".contains(c)) {
        return Err("Adresse invalide : saisissez l’adresse IP de l’imprimante (par exemple 192.168.1.50) ou son nom.".into());
    }
    // champ laissé vide : le mot de passe déjà enregistré est gardé
    let mot_de_passe = match mot_de_passe.as_str() {
        "" => config(&app)["mot_de_passe"].as_str().unwrap_or("").to_string(),
        _ => mot_de_passe,
    };
    ecrire_config(&app, &json!({ "adresse": adresse, "mot_de_passe": mot_de_passe }))?;
    Imprimante::reliee(&app)?.lire("/api/version")
}

/// La version (nom, firmware) et l’état (températures, impression) de l’imprimante, tels que PrusaLink les donne.
#[tauri::command(async)]
pub fn imprimante_etat(app: AppHandle) -> Result<Value, String> {
    let imprimante = Imprimante::reliee(&app)?;
    Ok(json!({ "version": imprimante.lire("/api/version")?, "statut": imprimante.lire("/api/v1/status")? }))
}

/// Les imprimantes Prusa annoncées sur le réseau local (service Bonjour de PrusaLink), écoutées 3 secondes.
#[tauri::command(async)]
pub fn imprimantes_chercher() -> Result<Vec<Value>, String> {
    let mdns = mdns_sd::ServiceDaemon::new().map_err(|e| format!("Recherche impossible : {e}"))?;
    let annonces = mdns.browse("_prusalink._tcp.local.").map_err(|e| format!("Recherche impossible : {e}"))?;
    let fin = Instant::now() + Duration::from_secs(3);
    let mut trouvees: Vec<Value> = vec![];
    while let Ok(evenement) = annonces.recv_deadline(fin) {
        let mdns_sd::ServiceEvent::ServiceResolved(s) = evenement else { continue };
        // l’IPv4, celle que l’imprimante affiche dans Réglages › Réseau
        let Some(ip) = s.get_addresses_v4().into_iter().next() else { continue };
        let adresse = if s.get_port() == 80 { ip.to_string() } else { format!("{ip}:{}", s.get_port()) };
        if !trouvees.iter().any(|t| t["adresse"] == adresse.as_str()) {
            trouvees.push(json!({ "nom": mdns_sd::instance_name_unescaped(s.get_fullname()), "adresse": adresse }));
        }
    }
    let _ = mdns.shutdown();
    Ok(trouvees)
}

/// Un nom que l’imprimante accepte : lettres, chiffres, « - », « _ » et « . » (le reste devient « _ »), extension
/// gardée, et une adresse de 168 octets au plus.
pub fn nom_sur(nom: &str, extension: &str) -> String {
    let base: String = nom.chars().map(|c| if c.is_ascii_alphanumeric() || "-_.".contains(c) { c } else { '_' }).collect();
    let place = 168 - USB.len() - 1 - extension.len();
    format!("{}.{extension}", &base[..base.len().min(place)])
}

/// 201 : reçu (et lancé si c’était demandé). 409 « Can't print right now » : reçu, mais pas lancé.
pub fn resultat_envoi((code, corps): (u16, String), nom: &str, imprimer: bool) -> Result<Value, String> {
    match code {
        201 => Ok(json!({ "nom": nom, "lance": imprimer })),
        409 if corps.contains("Can't print right now") => Ok(json!({ "nom": nom, "lance": false })),
        _ => Err(refus(code, &corps)),
    }
}

/// Les refus d’un envoi, d’après les réponses du firmware Buddy (gcode_upload.cpp).
pub fn refus(code: u16, corps: &str) -> String {
    match code {
        404 => "L’imprimante ne trouve pas sa clé USB : branchez-en une sur l’imprimante.".into(),
        409 if corps.contains("Another transfer") => "Un autre envoi vers l’imprimante est en cours : réessayez quand il sera fini.".into(),
        409 if corps.contains("busy") => "Ce fichier est en cours d’impression : il ne peut pas être remplacé.".into(),
        415 => "L’imprimante refuse ce type de fichier.".into(),
        507 => "La clé USB de l’imprimante est pleine, ou l’écriture y a échoué.".into(),
        _ => format!("L’imprimante a refusé l’envoi (code {code})."),
    }
}

/// Envoie un G-code tranché sur la clé de l’imprimante, qui l’imprime aussitôt.
#[tauri::command(async)]
pub fn envoyer_fichier(app: AppHandle, chemin: String) -> Result<Value, String> {
    let chemin = Path::new(&chemin);
    let extension = chemin.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    if extension != "gcode" && extension != "bgcode" {
        return Err("Choisissez un fichier .gcode ou .bgcode.".into());
    }
    let imprimante = Imprimante::reliee(&app)?;
    let fichier = fs::File::open(chemin).map_err(|e| format!("Fichier illisible : {e}"))?;
    let taille = fichier.metadata().map_err(|e| format!("Fichier illisible : {e}"))?.len();
    let nom = nom_sur(&chemin.file_stem().unwrap_or_default().to_string_lossy(), &extension);
    resultat_envoi(imprimante.deposer(&nom, fichier, taille, true)?, &nom, true)
}

/// Lance l’impression (ou la musique) d’un fichier déjà sur la clé de l’imprimante.
#[tauri::command(async)]
pub fn imprimer(app: AppHandle, nom: String) -> Result<(), String> {
    if nom.is_empty() || !nom.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)) {
        return Err("Nom de fichier invalide.".into());
    }
    match Imprimante::reliee(&app)?.requete("POST", &format!("{USB}{nom}"), &[], &b""[..], COURT)?.0 {
        200 | 204 => Ok(()),
        409 => Err("L’imprimante ne peut toujours pas le lancer : une impression est en cours, ou elle attend une action sur son écran.".into()),
        404 => Err("Ce fichier n’est plus sur la clé de l’imprimante.".into()),
        code => Err(format!("L’imprimante a refusé (code {code}).")),
    }
}

#[cfg(test)]
mod tests {
    use super::{nom_sur, USB};

    #[test]
    fn noms_acceptes_par_l_imprimante() {
        assert_eq!(nom_sur("Benchy (0.4) été", "bgcode"), "Benchy__0.4___t_.bgcode");
        let long = nom_sur(&"x".repeat(400), "gcode");
        assert!(long.ends_with(".gcode") && USB.len() + long.len() == 168, "{}", long.len());
    }
}
