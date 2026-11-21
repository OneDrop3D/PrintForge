/* Musique : un fichier MIDI devient un G-code que les moteurs de l’axe X « chantent ». Un moteur pas à pas sonne à la
 * fréquence de ses pas complets (les micro-pas, lissés par les pilotes TMC, s’entendent à peine) : pour une note de
 * f Hz, l’axe file à f / pas_par_mm mm/s pendant toute la note, pas_par_mm comptant les pas complets (6,25 sur la
 * CORE One : 100 micro-pas par mm, 16 micro-pas par pas). Compter les micro-pas faisait des mouvements 16 fois plus
 * courts, qu’on n’entendait presque pas. Une seule note à la fois (la plus aiguë), en grands allers-retours autour du
 * centre du plateau. */
use crate::firmware;
use crate::imprimante::{nom_sur, resultat_envoi, Imprimante};
use midly::{Format, MetaMessage, MidiMessage, Smf, Timing, TrackEventKind};
use serde_json::{json, Value};
use std::fmt::Write;
use std::path::Path;
use tauri::AppHandle;

/// Course permise de part et d’autre du centre, en micromètres : X de 25 à 225 mm sur la CORE One (250 mm).
const BORNE: i64 = 100_000;
/// Au-delà de cette vitesse (mm/s), la note descend d’une octave.
const VITESSE_MAX: f64 = 300.0;
/// Le centre du plateau de la CORE One (250 × 220 mm).
const CENTRE: &str = "X125 Y110";

/// Les notes à jouer : début et fin en secondes, numéro MIDI. À chaque instant, la plus aiguë de celles qui sonnent.
fn notes(octets: &[u8]) -> Result<Vec<(f64, f64, u8)>, String> {
    let smf = Smf::parse(octets).map_err(|e| format!("Fichier MIDI illisible ({e})."))?;
    if smf.header.format == Format::Sequential {
        return Err("Les fichiers MIDI de format 2 ne sont pas pris en charge.".into());
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
    let mut courante: Option<(f64, u8)> = None;
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
        let haute = (0..128u8).rev().find(|&k| sonnent[usize::from(k)] > 0);
        let change = match (courante, haute) {
            (Some((_, a)), Some(b)) => a != b || frappees[usize::from(b)],
            (a, b) => a.is_some() != b.is_some(),
        };
        if change {
            sortie.extend(courante.filter(|&(debut, _)| t > debut).map(|(debut, k)| (debut, t, k)));
            courante = haute.map(|k| (t, k));
        }
    }
    // une note jamais relâchée s’arrête avec le morceau
    sortie.extend(courante.filter(|&(debut, _)| t > debut).map(|(debut, k)| (debut, t, k)));
    Ok(sortie)
}

/// Le G-code d’un fichier MIDI : le texte, le nombre de notes et la durée en secondes.
pub fn gcode(octets: &[u8], pas_par_mm: f64) -> Result<(String, usize, f64), String> {
    if !(pas_par_mm.is_finite() && pas_par_mm > 0.0) {
        return Err("Le nombre de pas par millimètre doit être un nombre positif.".into());
    }
    let notes = notes(octets)?;
    let (debut_morceau, fin_morceau) = match (notes.first(), notes.last()) {
        (Some(premiere), Some(derniere)) => (premiere.0, derniere.1),
        _ => return Err("Ce fichier MIDI ne contient aucune note à jouer.".into()),
    };
    let mut g = format!(
        "; Musique OneDrop PrintForge : {} notes, {pas_par_mm} pas/mm\nG28\nG90\nG1 Z10 F600\nG1 {CENTRE} F6000\nG91\n",
        notes.len()
    );
    // la position autour du centre en micromètres entiers : aucune dérive, même sur des milliers de notes
    let (mut x, mut sens, mut fin_precedente) = (0i64, 1i64, debut_morceau);
    for &(debut, fin, cle) in &notes {
        let silence = ((debut - fin_precedente) * 1000.0).round();
        if silence >= 1.0 {
            writeln!(g, "G4 P{silence:.0}").unwrap();
        }
        fin_precedente = fin;
        let mut vitesse = 440.0 * 2f64.powf((f64::from(cle) - 69.0) / 12.0) / pas_par_mm;
        while vitesse > VITESSE_MAX {
            vitesse /= 2.0;
        }
        let mut reste = (vitesse * (fin - debut) * 1000.0).round() as i64;
        while reste > 0 {
            let place = if sens > 0 { BORNE - x } else { x + BORNE };
            if place == 0 {
                sens = -sens;
                continue;
            }
            let pas = reste.min(place);
            writeln!(g, "G1 X{:.3} F{:.1}", (sens * pas) as f64 / 1000.0, vitesse * 60.0).unwrap();
            x += sens * pas;
            reste -= pas;
        }
        sens = -sens; // chaque note repart dans l’autre sens : deux notes égales s’entendent séparées
    }
    g.push_str("G90\n");
    Ok((g, notes.len(), fin_morceau - debut_morceau))
}

/// Convertit le fichier MIDI puis l’envoie sur la clé de l’imprimante ; « jouer » le lance aussitôt.
#[tauri::command(async)]
pub fn musique_envoyer(app: AppHandle, chemin: String, pas_par_mm: f64, jouer: bool) -> Result<Value, String> {
    let imprimante = Imprimante::reliee(&app)?;
    let octets = std::fs::read(&chemin).map_err(|e| format!("Fichier illisible : {e}"))?;
    let (texte, notes, duree) = gcode(&octets, pas_par_mm)?;
    let nom = nom_musique(&chemin);
    let mut r = resultat_envoi(imprimante.deposer(&nom, texte.as_bytes(), texte.len() as u64, jouer)?, &nom, jouer)?;
    r["notes"] = notes.into();
    r["duree"] = duree.into();
    Ok(r)
}

/// Convertit le fichier MIDI et le met sur une clé USB de cet ordinateur, puis l’éjecte : branchée sur l’imprimante,
/// la musique se lance depuis ses fichiers. L’imprimante n’a pas besoin d’être reliée.
#[tauri::command(async)]
pub fn musique_cle(chemin: String, pas_par_mm: f64, cle: String) -> Result<Value, String> {
    firmware::cle_lisible(&cle)?;
    let octets = std::fs::read(&chemin).map_err(|e| format!("Fichier illisible : {e}"))?;
    let (texte, notes, duree) = gcode(&octets, pas_par_mm)?;
    let nom = nom_musique(&chemin);
    firmware::ecrire_sur_cle(&cle, &nom, texte.as_bytes())?;
    Ok(json!({ "nom": nom, "notes": notes, "duree": duree, "ejectee": firmware::ejecter(&cle) }))
}

/// Le même nom par le réseau ou par la clé : « Mon morceau.mid » → « musique-Mon_morceau.gcode ».
fn nom_musique(chemin: &str) -> String {
    nom_sur(&format!("musique-{}", Path::new(chemin).file_stem().unwrap_or_default().to_string_lossy()), "gcode")
}

#[cfg(test)]
mod tests {
    use super::gcode;

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

    /// Les déplacements (X en µm, F) et les pauses (ms) de la partie musicale du G-code.
    fn lire(g: &str) -> (Vec<(i64, f64)>, Vec<u64>) {
        let (mut mouvements, mut pauses) = (vec![], vec![]);
        for ligne in g.lines() {
            match ligne.split_whitespace().collect::<Vec<_>>().as_slice() {
                ["G1", x, f] if x.starts_with('X') => {
                    mouvements.push(((x[1..].parse::<f64>().unwrap() * 1000.0).round() as i64, f[1..].parse().unwrap()))
                }
                ["G4", p] => pauses.push(p[1..].parse().unwrap()),
                _ => {}
            }
        }
        (mouvements, pauses)
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
        // la4 = 440 Hz à 100 pas/mm : 4,4 mm/s, soit F264 ; une note sur deux dans l’autre sens
        assert_eq!(mouvements, vec![(2200, 264.0), (-2200, 264.0), (4400, 264.0)]);
        assert_eq!(pauses, vec![500]);
        assert_eq!(notes, 3);
        assert!((duree - 2.5).abs() < 1e-9, "{duree}");
        assert!(g.contains("\nG28\nG90\nG1 Z10 F600\nG1 X125 Y110 F6000\nG91\n") && g.ends_with("G90\n"), "{g}");
    }

    #[test]
    fn la_plus_aigue_seulement() {
        let do4: Piste = &[(0, &[0x90, 60, 100]), (960, &[0x80, 60, 0])]; // pendant 1 s
        let mi4: Piste = &[
            (0, &[0x99, 100, 100]),                         // percussion (canal 10) : ignorée malgré sa hauteur
            (480, &[0x90, 64, 100]), (240, &[0x90, 64, 0]), // de 0,5 à 0,75 s ; vélocité 0 = fin de note
        ];
        let (g, notes, _) = gcode(&midi(1, &[do4, mi4]), 100.0).unwrap();
        let (mouvements, pauses) = lire(&g);
        // do4 261,63 Hz → 2,616 mm/s ; mi4 329,63 Hz → 3,296 mm/s ; le do reprend quand le mi se tait
        assert_eq!(mouvements, vec![(1308, 157.0), (-824, 197.8), (654, 157.0)]);
        assert!(pauses.is_empty());
        assert_eq!(notes, 3);
    }

    #[test]
    fn reste_dans_les_bornes() {
        // la4 à 10 pas/mm : 44 mm/s ; deux notes de 3 s font chacune 132 mm, plus que les ±100 mm permis
        let piste: Piste = &[(0, LA_ON), (2880, LA_OFF), (0, LA_ON), (2880, LA_OFF)];
        let (g, ..) = gcode(&midi(0, &[piste]), 10.0).unwrap();
        let (mouvements, _) = lire(&g);
        let mut x = 0;
        for &(dx, _) in &mouvements {
            x += dx;
            assert!(x.abs() <= 100_000, "hors des bornes : {x} µm");
        }
        assert_eq!(mouvements.iter().map(|m| m.0).collect::<Vec<_>>(), vec![100_000, -32_000, 32_000, -100_000]);
    }

    #[test]
    fn pas_complets_de_la_core_one() {
        // la4 à 6,25 pas complets par mm : 70,4 mm/s, soit 35,2 mm pour une noire à 120 (0,5 s)
        let piste: Piste = &[(0, TEMPO_120), (0, LA_ON), (480, LA_OFF)];
        let (g, ..) = gcode(&midi(0, &[piste]), 6.25).unwrap();
        assert_eq!(lire(&g).0, vec![(35_200, 4224.0)]);
    }

    #[test]
    fn trop_rapide_descend_d_une_octave() {
        // la4 à 1 pas/mm voudrait 440 mm/s : il est joué une octave plus bas, à 220 mm/s, pendant 0,05 s
        let piste: Piste = &[(0, LA_ON), (48, LA_OFF)];
        let (g, ..) = gcode(&midi(0, &[piste]), 1.0).unwrap();
        assert_eq!(lire(&g).0, vec![(11_000, 13_200.0)]);
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
