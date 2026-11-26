/* Musique : un fichier MIDI devient un G-code que les moteurs « chantent ». Un moteur pas à pas sonne à la fréquence
 * de ses pas complets (les micro-pas, lissés par les pilotes TMC, s’entendent à peine) : pour une note de f Hz, il file
 * à f / pas_par_mm mm/s, pas_par_mm comptant les pas complets (6,25 sur la CORE One : 100 micro-pas par mm, 16
 * micro-pas par pas).
 *
 * Trois voix, comme le firmware OneDrop3D (music_od3d.cpp, même algorithme) : à chaque instant la mélodie (la plus
 * aiguë), une 2e voix (la 2e plus aiguë, s’il en sonne au moins 3) et la basse (la plus grave). Sur la CoreXY, les
 * moteurs du haut A (X+Y) et B (X−Y) chantent chacun leur voix dans un seul déplacement : vx = (vA+vB)/2,
 * vy = (vA−vB)/2 ; le plateau (Z) chante la basse. B sans voix double la mélodie (deux moteurs, plus fort). Un accord de
 * moins de 40 ms collé au précédent garde ses notes : un morceau joué par un humain n’a jamais deux notes frappées
 * pile ensemble. */
use crate::firmware;
use crate::imprimante::{nom_sur, resultat_envoi, Imprimante};
use midly::{Format, MetaMessage, MidiMessage, Smf, Timing, TrackEventKind};
use serde_json::{json, Value};
use std::fmt::Write;
use std::path::Path;
use tauri::AppHandle;

/// Mélodie, 2e voix, basse ; et les moteurs qui les chantent : A, B, Z.
const VOIX: usize = 3;
/// Course permise de part et d’autre du centre, en micromètres : |A|, |B| ≤ 100 mm donne |X|, |Y| ≤ 100 mm (X de 25 à
/// 225 mm sur la CORE One) ; Z de 10 à 40 mm.
const BORNES: [i64; VOIX] = [100_000, 100_000, 15_000];
/// Au-delà de ces vitesses (mm/s), la note descend d’une octave : moteurs du haut, puis Z.
const VITESSE_MAX: [f64; VOIX] = [300.0, 300.0, 20.0];
/// Pas complets par mm de Z sur la CORE One (400 micro-pas par mm, 16 par pas).
/// shortcut: valeur de la CORE One ; à demander à l’utilisateur si une autre imprimante joue de la musique
const PAS_Z: f64 = 25.0;
/// En dessous (Hz), un moteur ne fait que cliquer : la note monte d’une octave.
const FREQUENCE_MIN: f64 = 55.0;
/// Un accord plus court (s), collé au précédent, garde ses notes.
const ACCORD_MIN: f64 = 0.040;
/// Le centre du plateau de la CORE One (250 × 220 mm), et du jeu de Z.
const CENTRE: &str = "X125 Y110";
const CENTRE_Z: &str = "Z25";

/// Un accord : début et fin en secondes, la note de chaque voix (0 : muette), les voix qui frappent (bit v).
type Accord = (f64, f64, [u8; VOIX], u8);

/// Les voix d’un instant d’après les notes qui sonnent.
fn voix(sonnent: &[u32; 128]) -> [u8; VOIX] {
    let k: Vec<u8> = (1..128u8).rev().filter(|&k| sonnent[usize::from(k)] > 0).collect();
    [k.first().copied().unwrap_or(0), if k.len() >= 3 { k[1] } else { 0 }, if k.len() >= 2 { k[k.len() - 1] } else { 0 }]
}

/// Les accords à jouer, dans l’ordre du temps (les silences sont entre deux accords).
fn accords(octets: &[u8]) -> Result<Vec<Accord>, String> {
    let smf = Smf::parse(octets).map_err(|e| tr!("Fichier MIDI illisible ({e}).", "Unreadable MIDI file ({e})."))?;
    if smf.header.format == Format::Sequential {
        return Err(tr!("Les fichiers MIDI de format 2 ne sont pas pris en charge.", "Format 2 MIDI files are not supported."));
    }
    // (tic, sorte, valeur), toutes pistes fusionnées. À tic égal : tempo, fins de notes, débuts, fins de piste ;
    // une note relâchée puis frappée au même instant est donc rejouée.
    let mut evenements: Vec<(u64, u8, u32)> = vec![];
    for piste in &smf.tracks {
        let mut tic = 0;
        for e in piste {
            tic += u64::from(e.delta.as_int());
            match e.kind {
                TrackEventKind::Meta(MetaMessage::Tempo(t)) => evenements.push((tic, 0, t.as_int())),
                TrackEventKind::Meta(MetaMessage::EndOfTrack) => evenements.push((tic, 3, 0)),
                // le canal 10 est celui des percussions, sans hauteur de note
                TrackEventKind::Midi { channel, message } if channel.as_int() != 9 => match message {
                    MidiMessage::NoteOn { key, vel } if vel.as_int() > 0 => evenements.push((tic, 2, key.as_int().into())),
                    MidiMessage::NoteOn { key, .. } | MidiMessage::NoteOff { key, .. } => evenements.push((tic, 1, key.as_int().into())),
                    _ => {}
                },
                _ => {}
            }
        }
    }
    evenements.sort_by_key(|&(tic, sorte, _)| (tic, sorte));
    let duree_tic = |tempo: u32| match smf.header.timing {
        Timing::Metrical(division) => f64::from(tempo) / 1e6 / f64::from(division.as_int()),
        Timing::Timecode(images, sous_images) => 1.0 / (f64::from(images.as_f32()) * f64::from(sous_images)),
    };
    // 120 noires par minute tant que le fichier ne donne pas de tempo
    let (mut tempo, mut tic_precedent, mut t) = (500_000, 0, 0.0);
    let mut sonnent = [0u32; 128];
    let (mut courant, mut attaque_courante, mut debut) = ([0u8; VOIX], 0u8, 0.0);
    let mut sortie = vec![];
    let mut i = 0;
    while i < evenements.len() {
        let tic = evenements[i].0;
        t += (tic - tic_precedent) as f64 * duree_tic(tempo);
        tic_precedent = tic;
        let mut frappees = [false; 128];
        while let Some(&(_, sorte, valeur)) = evenements.get(i).filter(|e| e.0 == tic) {
            match sorte {
                0 => tempo = valeur,
                1 => sonnent[valeur as usize] = sonnent[valeur as usize].saturating_sub(1),
                2 => {
                    sonnent[valeur as usize] += 1;
                    frappees[valeur as usize] = true;
                }
                _ => {}
            }
            i += 1;
        }
        let cles = voix(&sonnent);
        // une voix frappe si sa note change ou est refrappée
        let attaque = (0..VOIX).filter(|&v| cles[v] != 0 && (cles[v] != courant[v] || frappees[usize::from(cles[v])])).fold(0, |a, v| a | 1 << v);
        if attaque != 0 || cles != courant {
            if courant[0] != 0 && t > debut {
                sortie.push((debut, t, courant, attaque_courante));
            }
            (courant, attaque_courante, debut) = (cles, attaque, t);
        }
    }
    // une note jamais relâchée s’arrête avec le morceau
    if courant[0] != 0 && t > debut {
        sortie.push((debut, t, courant, attaque_courante));
    }
    Ok(sortie)
}

/// Lissage : un accord plus court que ACCORD_MIN collé au précédent garde les notes jouées ; une voix ne repart que si
/// sa note change vraiment ou si le fichier la refrappe (la même note qu’à l’accord lu juste avant).
fn lisser(accords: Vec<Accord>) -> Vec<Accord> {
    let (mut joue, mut lu): (Option<Accord>, Option<Accord>) = (None, None);
    accords
        .into_iter()
        .map(|a| {
            let colle = joue.filter(|j| j.2[0] != 0 && j.1 == a.0);
            let mut o = a;
            if let Some(j) = colle.filter(|_| a.1 - a.0 < ACCORD_MIN) {
                (o.2, o.3) = (j.2, 0);
            } else {
                o.3 = (0..VOIX)
                    .filter(|&v| {
                        let avant = colle.map_or(0, |j| j.2[v]);
                        let refrappee = a.3 & (1 << v) != 0 && lu.is_some_and(|l| l.2[v] == a.2[v]);
                        a.2[v] != 0 && (a.2[v] != avant || refrappee)
                    })
                    .fold(0, |x, v| x | 1 << v);
            }
            (lu, joue) = (Some(a), Some(o));
            o
        })
        .collect()
}

/// Vitesse d’un moteur (mm/s) pour une note : ramenée sous vmax et au-dessus de FREQUENCE_MIN par octaves.
fn vitesse_note(cle: u8, pas_par_mm: f64, vmax: f64) -> f64 {
    let mut v = 440.0 * 2f64.powf((f64::from(cle) - 69.0) / 12.0) / pas_par_mm;
    while v > vmax {
        v /= 2.0;
    }
    while v < FREQUENCE_MIN / pas_par_mm && v * 2.0 <= vmax {
        v *= 2.0;
    }
    v
}

/// Prochain morceau d’un accord, un seul déplacement pour toutes les voix (comme chord_step du firmware, sans sa
/// limite de 0,2 s qui ne sert qu’à arrêter vite) : rend sa durée et les déplacements en µm ; une voix qui touche sa
/// borne repart dans l’autre sens au morceau suivant.
fn morceau(pos: &mut [i64; VOIX], sens: &mut [i64; VOIX], vitesse: &[f64; VOIX], reste: f64) -> (f64, [i64; VOIX]) {
    let mut dt = reste;
    let mut place = [0i64; VOIX];
    let mut limite = None;
    for v in (0..VOIX).filter(|&v| vitesse[v] > 0.0) {
        let place_de = |s: i64| if s > 0 { BORNES[v] - pos[v] } else { pos[v] + BORNES[v] };
        if place_de(sens[v]) <= 0 {
            sens[v] = -sens[v];
        }
        place[v] = place_de(sens[v]);
        if (place[v] as f64) / vitesse[v] < dt {
            dt = place[v] as f64 / vitesse[v];
            limite = Some(v);
        }
    }
    let mut d = [0i64; VOIX];
    for v in (0..VOIX).filter(|&v| vitesse[v] > 0.0) {
        let mut long = if limite == Some(v) { place[v] } else { place[v].min((vitesse[v] * dt).round() as i64) };
        if place[v] - long <= 1 {
            long = place[v]; // à 1 µm près, à la borne : deux voix égales restent ensemble
        }
        d[v] = sens[v] * long;
        pos[v] += d[v];
    }
    (dt, d)
}

/// Le G-code d’un fichier MIDI : le texte, le nombre de notes frappées et la durée en secondes.
pub fn gcode(octets: &[u8], pas_par_mm: f64) -> Result<(String, usize, f64), String> {
    if !(pas_par_mm.is_finite() && pas_par_mm > 0.0) {
        return Err(tr!("Le nombre de pas par millimètre doit être un nombre positif.", "Steps per millimetre must be a positive number."));
    }
    let accords = lisser(accords(octets)?);
    let (debut_morceau, fin_morceau) = match (accords.first(), accords.last()) {
        (Some(premier), Some(dernier)) => (premier.0, dernier.1),
        _ => return Err(tr!("Ce fichier MIDI ne contient aucune note à jouer.", "This MIDI file has no notes to play.")),
    };
    let notes: usize = accords.iter().map(|a| a.3.count_ones() as usize).sum();
    // M862.6 « Input shaper » : comme un G-code de PrusaSlicer, sans quoi l’aperçu de l’imprimante avertit à tort
    let mut g = format!(
        "; Musique OneDrop PrintForge : {notes} notes, 3 voix, {pas_par_mm} pas/mm\nM862.6 P \"Input shaper\"\nG28\nG90\nG1 {CENTRE_Z} F600\nG1 {CENTRE} F6000\nG91\n"
    );
    // les positions autour du centre en micromètres entiers : aucune dérive, même sur des milliers de notes
    // sens de départ −1 : la première note, qui frappe, part vers +X
    let (mut pos, mut sens, mut fin_precedente) = ([0i64; VOIX], [-1i64; VOIX], debut_morceau);
    let pas = [pas_par_mm, pas_par_mm, PAS_Z];
    for &(debut, fin, cles, attaque) in &accords {
        let silence = ((debut - fin_precedente) * 1000.0).round();
        if silence >= 1.0 {
            writeln!(g, "G4 P{silence:.0}").unwrap();
        }
        fin_precedente = fin;
        // B sans voix double la mélodie
        let (mut moteurs, mut frappe) = (cles, attaque);
        if moteurs[1] == 0 {
            moteurs[1] = moteurs[0];
            frappe = (frappe & !2) | ((frappe & 1) << 1);
        }
        let vitesse: [f64; VOIX] = std::array::from_fn(|v| if moteurs[v] == 0 { 0.0 } else { 1000.0 * vitesse_note(moteurs[v], pas[v], VITESSE_MAX[v]) });
        for (v, s) in sens.iter_mut().enumerate() {
            if frappe & (1 << v) != 0 {
                *s = -*s; // une note frappée repart dans l’autre sens : elle s’entend
            }
        }
        let mut reste = fin - debut;
        while reste > 1e-6 {
            let (dt, d) = morceau(&mut pos, &mut sens, &vitesse, reste);
            if dt <= 0.0 {
                break;
            }
            reste -= dt;
            if d == [0; VOIX] {
                continue;
            }
            // CoreXY : A = X + Y, B = X − Y ; une seule ligne, sa vitesse donne à chaque moteur la sienne
            let (x, y, z) = ((d[0] + d[1]) as f64 / 2000.0, (d[0] - d[1]) as f64 / 2000.0, d[2] as f64 / 1000.0);
            let mut ligne = String::from("G1");
            for (axe, valeur) in [('X', x), ('Y', y), ('Z', z)] {
                if valeur != 0.0 {
                    write!(ligne, " {axe}{valeur:.4}").unwrap();
                }
            }
            writeln!(g, "{ligne} F{:.1}", (x * x + y * y + z * z).sqrt() / dt * 60.0).unwrap();
        }
    }
    g.push_str("G90\n");
    Ok((g, notes, fin_morceau - debut_morceau))
}

/// Convertit le fichier MIDI puis l’envoie sur la clé de l’imprimante ; « jouer » le lance aussitôt.
#[tauri::command(async)]
pub fn musique_envoyer(app: AppHandle, chemin: String, pas_par_mm: f64, jouer: bool) -> Result<Value, String> {
    let imprimante = Imprimante::reliee(&app)?;
    let octets = std::fs::read(&chemin).map_err(|e| tr!("Fichier illisible : {e}", "Unreadable file: {e}"))?;
    let (texte, notes, duree) = gcode(&octets, pas_par_mm)?;
    let nom = nom_musique(&chemin, "gcode");
    let mut r = resultat_envoi(imprimante.deposer(&nom, texte.as_bytes(), texte.len() as u64, jouer)?, &nom, jouer)?;
    r["notes"] = notes.into();
    r["duree"] = duree.into();
    Ok(r)
}

/// Convertit le fichier MIDI et le met sur une clé USB de cet ordinateur avec le fichier MIDI lui-même (l’écran
/// Contrôle › Musique des moteurs de l’imprimante le joue), puis l’éjecte. L’imprimante n’a pas besoin d’être reliée.
#[tauri::command(async)]
pub fn musique_cle(chemin: String, pas_par_mm: f64, cle: String) -> Result<Value, String> {
    firmware::cle_lisible(&cle)?;
    let octets = std::fs::read(&chemin).map_err(|e| tr!("Fichier illisible : {e}", "Unreadable file: {e}"))?;
    let (texte, notes, duree) = gcode(&octets, pas_par_mm)?;
    let (nom, midi) = (nom_musique(&chemin, "gcode"), nom_musique(&chemin, "mid"));
    firmware::ecrire_sur_cle(&cle, &nom, texte.as_bytes())?;
    firmware::ecrire_sur_cle(&cle, &midi, &octets)?;
    Ok(json!({ "nom": nom, "midi": midi, "notes": notes, "duree": duree, "ejectee": firmware::ejecter(&cle) }))
}

/// Le même nom par le réseau ou par la clé : « Mon morceau.mid » → « musique-Mon_morceau.gcode » (ou « .mid »).
fn nom_musique(chemin: &str, extension: &str) -> String {
    nom_sur(&format!("musique-{}", Path::new(chemin).file_stem().unwrap_or_default().to_string_lossy()), extension)
}

#[cfg(test)]
mod tests {
    use super::{accords, gcode, lisser, Accord, PAS_Z};

    type Piste<'a> = &'a [(u32, &'a [u8])];
    const TEMPO_120: &[u8] = &[0xFF, 0x51, 3, 0x07, 0xA1, 0x20]; // 500 000 µs par noire
    const TEMPO_60: &[u8] = &[0xFF, 0x51, 3, 0x0F, 0x42, 0x40]; // 1 000 000 µs par noire
    const LA_ON: &[u8] = &[0x90, 69, 100];
    const LA_OFF: &[u8] = &[0x80, 69, 0];

    /// Un fichier MIDI à 480 tics par noire ; chaque piste est une suite de (delta en tics, octets de l’événement).
    fn midi(format: u16, pistes: &[Piste]) -> Vec<u8> {
        let mut m = b"MThd\0\0\0\x06".to_vec();
        for v in [format, pistes.len() as u16, 480] {
            m.extend(v.to_be_bytes());
        }
        for piste in pistes {
            let mut corps = vec![];
            for &(delta, octets) in *piste {
                // quantité de longueur variable : 7 bits par octet, le bit fort annonce l’octet suivant
                let mut vlq = vec![(delta & 0x7F) as u8];
                let mut reste = delta >> 7;
                while reste > 0 {
                    vlq.insert(0, (reste & 0x7F) as u8 | 0x80);
                    reste >>= 7;
                }
                corps.extend(vlq);
                corps.extend(octets);
            }
            corps.extend([0, 0xFF, 0x2F, 0]);
            m.extend(b"MTrk");
            m.extend((corps.len() as u32).to_be_bytes());
            m.extend(corps);
        }
        m
    }

    /// Les déplacements (X, Y, Z en µm, F) et les pauses (ms) de la partie musicale du G-code.
    fn lire(g: &str) -> (Vec<([i64; 3], f64)>, Vec<u64>) {
        let (mut mouvements, mut pauses) = (vec![], vec![]);
        for ligne in g.lines().filter(|l| !l.starts_with(';')) {
            let mots: Vec<&str> = ligne.split_whitespace().collect();
            match mots.as_slice() {
                ["G1", reste @ ..] if reste.last().is_some_and(|f| f.starts_with('F')) => {
                    let mut d = [0i64; 3];
                    for m in &reste[..reste.len() - 1] {
                        let i = "XYZ".find(&m[..1]).unwrap();
                        d[i] = (m[1..].parse::<f64>().unwrap() * 1000.0).round() as i64;
                    }
                    mouvements.push((d, reste.last().unwrap()[1..].parse().unwrap()));
                }
                ["G4", p] => pauses.push(p[1..].parse().unwrap()),
                _ => {}
            }
        }
        // les deux premiers G1 (centre de Z, puis du plateau) ne sont pas de la musique
        (mouvements.into_iter().skip(2).collect(), pauses)
    }

    /// Les X d’une suite de déplacements qui ne vont qu’en X.
    fn en_x(mouvements: &[([i64; 3], f64)]) -> Vec<(i64, f64)> {
        mouvements.iter().map(|&(d, f)| {
            assert_eq!((d[1], d[2]), (0, 0), "{d:?}");
            (d[0], f)
        }).collect()
    }

    #[test]
    fn tempo_silences_et_alternance() {
        let piste: Piste = &[
            (0, TEMPO_120), (0, LA_ON), (480, LA_OFF), // 0,5 s
            (0, LA_ON), (480, LA_OFF),                 // rejouée aussitôt : 0,5 s
            (0, TEMPO_60), (240, LA_ON), (480, LA_OFF), // 0,5 s de silence au nouveau tempo, puis 1 s
        ];
        let (g, notes, duree) = gcode(&midi(0, &[piste]), 100.0).unwrap();
        let (mouvements, pauses) = lire(&g);
        // la4 = 440 Hz à 100 pas/mm : 4,4 mm/s, soit F264 ; une seule voix : A et B ensemble, un déplacement en X ;
        // une note sur deux dans l’autre sens
        assert_eq!(en_x(&mouvements), vec![(2200, 264.0), (-2200, 264.0), (4400, 264.0)]);
        assert_eq!(pauses, vec![500]);
        assert_eq!(notes, 3); // les notes du fichier (B qui double la mélodie ne compte pas)
        assert!((duree - 2.5).abs() < 1e-9, "{duree}");
        assert!(g.contains("\nM862.6 P \"Input shaper\"\nG28\nG90\nG1 Z25 F600\nG1 X125 Y110 F6000\nG91\n") && g.ends_with("G90\n"), "{g}");
    }

    #[test]
    fn melodie_et_basse() {
        let do4: Piste = &[(0, &[0x90, 60, 100]), (960, &[0x80, 60, 0])]; // pendant 1 s
        let mi4: Piste = &[
            (0, &[0x99, 100, 100]),                         // percussion (canal 10) : ignorée malgré sa hauteur
            (480, &[0x90, 64, 100]), (240, &[0x90, 64, 0]), // de 0,5 à 0,75 s ; vélocité 0 = fin de note
        ];
        let (g, _, _) = gcode(&midi(1, &[do4, mi4]), 100.0).unwrap();
        let (mouvements, pauses) = lire(&g);
        assert!(pauses.is_empty());
        // do4 seul : A et B à 2,616 mm/s, en X ; puis mi4 en mélodie (A et B) et do4 tenu à la basse, sur Z
        assert_eq!(mouvements[0], ([1308, 0, 0], 157.0));
        let (d, f) = mouvements[1];
        assert_eq!(d[0], -824); // mi4 : 3,296 mm/s pendant 0,25 s, dans l’autre sens
        assert_eq!(d[1], 0);
        let dt = ((d[0] * d[0] + d[2] * d[2]) as f64).sqrt() / 1000.0 / (f / 60.0);
        assert!((dt - 0.25).abs() < 1e-3, "{dt}");
        assert!(((d[2].abs() as f64 / 1000.0 / dt) * PAS_Z - 261.63).abs() < 1.0, "{d:?}"); // do4 sur Z
        assert_eq!(mouvements[2].0[2], 0); // le mi se tait : le do reprend la mélodie, Z s’arrête
    }

    #[test]
    fn trois_voix_dans_un_deplacement() {
        // do4-mi4-sol4 tenus 1 s : sol4 sur A, mi4 sur B, do4 sur Z
        let accord: Piste = &[(0, &[0x90, 60, 100]), (0, &[0x90, 64, 100]), (0, &[0x90, 67, 100]), (960, &[0x80, 60, 0]), (0, &[0x80, 64, 0]), (0, &[0x80, 67, 0])];
        let (g, notes, _) = gcode(&midi(0, &[accord]), 6.25).unwrap();
        assert_eq!(notes, 3);
        let (mouvements, _) = lire(&g);
        let (mut a, mut b, mut z, mut t) = (0.0, 0.0, 0.0, 0.0);
        for &(d, f) in &mouvements {
            let dist = ((d[0] * d[0] + d[1] * d[1] + d[2] * d[2]) as f64).sqrt() / 1000.0;
            t += dist / (f / 60.0);
            // A = X + Y, B = X − Y
            a += ((d[0] + d[1]) as f64).abs() / 1000.0;
            b += ((d[0] - d[1]) as f64).abs() / 1000.0;
            z += (d[2] as f64).abs() / 1000.0;
        }
        assert!((t - 1.0).abs() < 1e-2, "{t}");
        assert!((a / t * 6.25 - 392.0).abs() < 2.0, "sol4 : {}", a / t * 6.25);
        assert!((b / t * 6.25 - 329.6).abs() < 2.0, "mi4 : {}", b / t * 6.25);
        assert!((z / t * PAS_Z - 261.6).abs() < 2.0, "do4 : {}", z / t * PAS_Z);
    }

    #[test]
    fn lissage_des_accords_trop_courts() {
        let a = |debut: f64, fin: f64, cle: u8, attaque: u8| -> Accord { (debut, fin, [cle, 0, 0], attaque) };
        let o = lisser(vec![a(0.0, 0.5, 76, 1), a(0.5, 0.523, 64, 1), a(0.523, 0.9, 76, 1), a(0.9, 1.2, 76, 1), a(1.5, 1.51, 60, 1)]);
        let vu: Vec<(u8, u8)> = o.iter().map(|o| (o.2[0], o.3)).collect();
        // 23 ms d’accompagnement : le mi6 continue sans repartir ; refrappé ensuite par le fichier : il repart ; après un
        // silence, même très courte, la note est jouée
        assert_eq!(vu, vec![(76, 1), (76, 0), (76, 0), (76, 1), (60, 1)]);
    }

    #[test]
    fn voix_d_un_accord() {
        // do3-do4-mi4-sol4 : la 4e (do4) se tait ; do3 est la basse
        let piste: Piste = &[(0, &[0x90, 48, 100]), (0, &[0x90, 60, 100]), (0, &[0x90, 64, 100]), (0, &[0x90, 67, 100]), (480, &[0x80, 48, 0])];
        let a = accords(&midi(0, &[piste])).unwrap();
        assert_eq!(a[0].2, [67, 64, 48]);
    }

    #[test]
    fn reste_dans_les_bornes() {
        // la4 à 10 pas/mm : 44 mm/s ; deux notes de 3 s font chacune 132 mm, plus que les ±100 mm permis
        let piste: Piste = &[(0, LA_ON), (2880, LA_OFF), (0, LA_ON), (2880, LA_OFF)];
        let (g, ..) = gcode(&midi(0, &[piste]), 10.0).unwrap();
        let (mouvements, _) = lire(&g);
        let mut x = 0;
        for &(d, _) in &mouvements {
            x += d[0];
            assert!(x.abs() <= 100_000, "hors des bornes : {x} µm");
        }
        assert_eq!(en_x(&mouvements).iter().map(|m| m.0).collect::<Vec<_>>(), vec![100_000, -32_000, 32_000, -100_000]);
    }

    #[test]
    fn pas_complets_de_la_core_one() {
        // la4 à 6,25 pas complets par mm : 70,4 mm/s, soit 35,2 mm pour une noire à 120 (0,5 s)
        let piste: Piste = &[(0, TEMPO_120), (0, LA_ON), (480, LA_OFF)];
        let (g, ..) = gcode(&midi(0, &[piste]), 6.25).unwrap();
        assert_eq!(en_x(&lire(&g).0), vec![(35_200, 4224.0)]);
    }

    #[test]
    fn trop_rapide_descend_d_une_octave() {
        // la4 à 1 pas/mm voudrait 440 mm/s : il est joué une octave plus bas, à 220 mm/s, pendant 0,05 s
        let piste: Piste = &[(0, LA_ON), (48, LA_OFF)];
        let (g, ..) = gcode(&midi(0, &[piste]), 1.0).unwrap();
        assert_eq!(en_x(&lire(&g).0), vec![(11_000, 13_200.0)]);
    }

    #[test]
    fn fichiers_refuses() {
        let piste: Piste = &[(0, LA_ON), (480, LA_OFF)];
        let vide: Piste = &[];
        assert!(gcode(&midi(2, &[piste]), 100.0).is_err()); // format 2
        assert!(gcode(&midi(0, &[vide]), 100.0).is_err()); // aucune note
        assert!(gcode(&midi(0, &[piste]), 0.0).is_err());
        assert!(gcode(b"pas un fichier MIDI", 100.0).is_err());
    }
}
