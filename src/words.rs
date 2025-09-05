use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
};

use rand::{
    distr::{Distribution, weighted::WeightedIndex},
    rng,
};

pub fn build_dict(path: impl AsRef<std::path::Path>) -> anyhow::Result<HashMap<String, f64>> {
    let words: Vec<String> = std::fs::read_to_string(path)?
        .lines()
        .map(|s| s.trim().to_lowercase())
        .filter(|w| w.len() == 5 && w.chars().all(|c| c.is_ascii_lowercase()))
        .collect();

    let stats = compute_stats(&words);
    let wt = Weights::default();
    Ok(words
        .into_iter()
        .map(|w| {
            let s = score_word(&w, &stats, wt);
            (w, s)
        })
        .collect())
}

pub fn pick_weighted<'a>(
    dict: &'a HashMap<String, f64>,
    exclude: Option<&HashSet<String>>,
    alpha: Option<f64>,
) -> Option<&'a str> {
    let eps = 1e-6_f64;

    let mut keys: Vec<&str> = Vec::with_capacity(dict.len());
    let mut weights: Vec<f64> = Vec::with_capacity(dict.len());

    for (w, &s) in dict {
        if exclude.is_some_and(|ex| ex.contains(w)) {
            continue;
        }
        let mut wt = s.max(0.0) + eps; // ensure positive
        if let Some(alpha) = alpha {
            wt = wt.powf(alpha);
        }
        if wt.is_finite() && wt > 0.0 {
            keys.push(w.as_str());
            weights.push(wt);
        }
    }
    if keys.is_empty() {
        return None;
    }

    let distribution = WeightedIndex::new(&weights).ok()?;
    let mut rng = rng();
    let idx = distribution.sample(&mut rng);
    Some(keys[idx])
}

#[allow(dead_code)]
pub fn print_top(dict: &HashMap<String, f64>, n: usize, top: bool) {
    let mut v: Vec<(&str, f64)> = dict.iter().map(|(w, s)| (w.as_str(), *s)).collect();
    v.sort_by(|a, b| {
        // flip only the score comparison when top == true
        let (lhs, rhs) = if top { (b, a) } else { (a, b) };
        match lhs.1.partial_cmp(&rhs.1).unwrap_or(Ordering::Equal) {
            Ordering::Equal => a.0.cmp(b.0), // tie-break: word asc
            ord => ord,
        }
    });

    for (i, (w, s)) in v.into_iter().take(n).enumerate() {
        println!("{:>3}. {:8.3}  {}", i + 1, s, w);
    }
}

struct Stats {
    letter_ct: HashMap<char, usize>,
    bigram_ct: HashMap<(char, char), usize>,
    total_letters: f64,
    total_bigrams: f64,
}

fn compute_stats(words: &[String]) -> Stats {
    let mut letter_ct = HashMap::new();
    let mut bigram_ct = HashMap::new();
    for w in words {
        let chars: Vec<char> = w.chars().collect();
        for &c in &chars {
            *letter_ct.entry(c).or_default() += 1;
        }
        for i in 0..4 {
            *bigram_ct.entry((chars[i], chars[i + 1])).or_default() += 1;
        }
    }
    Stats {
        total_letters: (words.len() as f64) * 5.0,
        total_bigrams: (words.len() as f64) * 4.0,
        letter_ct,
        bigram_ct,
    }
}

#[derive(Clone, Copy)]
struct Weights {
    // corpus
    pub rare_letter: f64, // ln(1/freq) per letter
    pub rare_boost: f64,  // extra for jqxzkvwy per letter
    pub rare_bigram: f64, // ln(1/freq) per bigram

    // shared/local features
    pub no_vowels_y: f64, // no AEIOUY
    pub no_vowels: f64,   // no AEIOU (Y allowed)
    pub low_vowel_ratio: f64,
    pub adj_double: f64, // adjacent doubles count
    pub max_cons_cluster: f64,
    pub dup_extra: f64, // per extra letter occurrence (non-adjacent repeats)
    pub low_unique: f64,
    pub ababa: f64, // ABABA pattern
    pub repeated_bigram: f64,
    pub q_without_u: f64,
}

impl Default for Weights {
    fn default() -> Self {
        Weights {
            rare_letter: 0.35,
            rare_boost: 0.25,
            rare_bigram: 0.20,
            no_vowels_y: 9.0,
            no_vowels: 5.0,
            low_vowel_ratio: 2.0,
            adj_double: 1.0,
            max_cons_cluster: 1.0,
            dup_extra: 1.6,
            low_unique: 0.7,
            ababa: 3.0,
            repeated_bigram: 1.2,
            q_without_u: 2.0,
        }
    }
}

fn score_word(word: &str, stats: &Stats, wt: Weights) -> f64 {
    let b = word.as_bytes();
    let eps = 1e-6_f64;
    let rare = [b'j', b'q', b'x', b'z', b'k', b'v', b'w', b'y'];

    // vowels
    let has_v = b
        .iter()
        .any(|&c| matches!(c, b'a' | b'e' | b'i' | b'o' | b'u'));
    let has_vy = b
        .iter()
        .any(|&c| matches!(c, b'a' | b'e' | b'i' | b'o' | b'u' | b'y'));
    let vowel_ratio = b
        .iter()
        .filter(|&&c| matches!(c, b'a' | b'e' | b'i' | b'o' | b'u'))
        .count() as f64
        / 5.0;

    // counts and repeats
    let mut cnt = [0u8; 26];
    for &c in b {
        cnt[(c - b'a') as usize] += 1;
    }
    let unique = i32::try_from(cnt.iter().filter(|&&k| k > 0).count())
        .expect("Score word failed, cnt value too large");
    let dup_total: i32 = cnt.iter().map(|&k| i32::from(k.saturating_sub(1))).sum();

    // adjacent doubles
    let adj_doubles = (0..4).filter(|&i| b[i] == b[i + 1]).count() as f64;

    // max consonant cluster (y treated as vowel)
    let mut best = 0;
    let mut cur = 0;
    for &c in b {
        match c {
            b'a' | b'e' | b'i' | b'o' | b'u' | b'y' => cur = 0,
            _ => {
                cur += 1;
                best = best.max(cur);
            }
        }
    }

    // ABABA pattern (0=2=4 and 1=3, and a!=b)
    let ababa = f64::from(i32::from(
        b[0] == b[2] && b[2] == b[4] && b[0] != b[1] && b[1] == b[3],
    ));

    // repeated bigrams inside the word
    let bigrams = [(b[0], b[1]), (b[1], b[2]), (b[2], b[3]), (b[3], b[4])];
    let mut seen = std::collections::HashSet::new();
    let mut repeated_bg = 0f64;
    for &bg in &bigrams {
        if !seen.insert(bg) {
            repeated_bg += 1.0;
        }
    }

    // Q without U
    let q_without_u = if word.contains('q') && !word.contains('u') {
        1.0
    } else {
        0.0
    };

    // corpus rarity (letters + bigrams)
    let mut rare_letter_score = 0.0;
    for &bb in b {
        let c = bb as char;
        let f = (*stats.letter_ct.get(&c).unwrap_or(&1) as f64 / stats.total_letters).max(eps);
        rare_letter_score += (1.0 / f).ln()
            + if rare.contains(&bb) {
                wt.rare_boost
            } else {
                0.0
            };
    }
    let mut rare_bigram_score = 0.0;
    for i in 0..4 {
        let k = (b[i] as char, b[i + 1] as char);
        let f = (*stats.bigram_ct.get(&k).unwrap_or(&1) as f64 / stats.total_bigrams).max(eps);
        rare_bigram_score += (1.0 / f).ln();
    }

    // combine
    let mut score = 0.0;
    if !has_vy {
        score += wt.no_vowels_y;
    } else if !has_v {
        score += wt.no_vowels;
    }
    if vowel_ratio < 0.2 {
        score += wt.low_vowel_ratio;
    }

    score += wt.rare_letter * rare_letter_score;
    score += wt.rare_bigram * rare_bigram_score;
    score += wt.adj_double * adj_doubles;
    score += wt.max_cons_cluster * f64::from(best);
    score += wt.dup_extra * f64::from(dup_total);
    score += wt.low_unique * f64::from((5 - unique).max(0));
    score += wt.ababa * ababa;
    score += wt.repeated_bigram * repeated_bg;
    score += wt.q_without_u * q_without_u;

    score
}
