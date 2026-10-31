//! `cargo run --release --bin depth -- [--seeds N] [--ladder random,greedy,search:100] [--rules FILE]`
//!
//! Plays every rung of the ladder on the same seeds and prints how each
//! does and how often each beats the rung below it. Depth D is the number of
//! steps in a row where the stronger rung wins at least 60% of the seeds;
//! the gap G is how often the top rung beats greedy.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use magistral_rules::ai::{play, Greedy, Player, Quick, Random, Search};
use magistral_rules::{Card, Config, Move, Rng, State};
use rayon::prelude::*;

struct Run {
    score: i64,
    stage: u8,
    won: bool,
    earned: i32,
    picks: Vec<Card>,
}

fn player(spec: &str, seed: u64) -> Box<dyn Player> {
    match spec.split_once(':') {
        Some(("search", n)) => Box::new(Search::new(n.parse().expect("search:N"), seed ^ 0xABCD)),
        _ => match spec {
            "random" => Box::new(Random(Rng::new(seed ^ 0x1234))),
            "quick" => Box::new(Quick(Rng::new(seed ^ 0x5678))),
            "greedy" => Box::new(Greedy),
            _ => panic!("no player `{spec}`: random, quick, greedy, search:N"),
        },
    }
}

fn run(cfg: &Arc<Config>, spec: &str, seed: u64) -> Run {
    let mut p = player(spec, seed);
    let mut state = State::new(cfg.clone(), seed);
    let mut picks = Vec::new();
    while !state.over() {
        let mv = p.choose(&state);
        if let Move::Pick(Some(c)) = mv {
            picks.push(c);
        }
        state.apply(mv);
    }
    let _ = play;
    Run {
        score: state.score(),
        stage: state.stage,
        won: state.won,
        earned: state.earned,
        picks,
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut seeds = 200u64;
    let mut ladder = "random,greedy,search:10,search:30,search:100,search:300".to_string();
    let mut rules = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--seeds" => seeds = args[i + 1].parse().expect("--seeds N"),
            "--ladder" => ladder = args[i + 1].clone(),
            "--rules" => rules = Some(args[i + 1].clone()),
            other => panic!("unknown argument {other}"),
        }
        i += 2;
    }
    let cfg = match rules {
        Some(path) => Config::parse(&std::fs::read_to_string(&path).expect("read rules"))
            .expect("parse rules"),
        None => Config::default(),
    };
    let cfg = Arc::new(cfg);
    let rungs: Vec<&str> = ladder.split(',').collect();
    let mut results: Vec<Vec<Run>> = Vec::new();
    println!(
        "{seeds} seeds, {} stages, payments {:?}\n",
        cfg.payments.len(),
        cfg.payments
    );
    println!(
        "{:<12} {:>7} {:>6} {:>8} {:>8}",
        "player", "stages", "won%", "earned", "seconds"
    );
    for spec in &rungs {
        let start = Instant::now();
        let runs: Vec<Run> = (0..seeds)
            .into_par_iter()
            .map(|seed| run(&cfg, spec, seed))
            .collect();
        let n = runs.len() as f64;
        let stages = runs.iter().map(|r| r.stage as f64).sum::<f64>() / n;
        let won = runs.iter().filter(|r| r.won).count() as f64 / n * 100.0;
        let earned = runs.iter().map(|r| r.earned as f64).sum::<f64>() / n;
        println!(
            "{:<12} {:>7.2} {:>6.1} {:>8.1} {:>8.1}",
            spec,
            stages,
            won,
            earned,
            start.elapsed().as_secs_f64()
        );
        results.push(runs);
    }
    let beats = |hi: &[Run], lo: &[Run]| -> f64 {
        let mut w = 0.0;
        for (h, l) in hi.iter().zip(lo) {
            if h.score > l.score {
                w += 1.0;
            } else if h.score == l.score {
                w += 0.5;
            }
        }
        w / hi.len() as f64
    };
    println!("\nladder (how often the stronger beats the one below, same seed):");
    let mut depth = 0;
    let mut counting = true;
    for i in 1..results.len() {
        let p = beats(&results[i], &results[i - 1]);
        let step = p >= 0.6;
        if counting && step {
            depth += 1;
        } else {
            counting = false;
        }
        println!(
            "  {:<12} > {:<12} {:>5.1}%{}",
            rungs[i],
            rungs[i - 1],
            p * 100.0,
            if step { "  step" } else { "" }
        );
    }
    if let Some(g) = rungs.iter().position(|r| *r == "greedy") {
        let top = results.len() - 1;
        if top != g {
            println!(
                "\nG: {} beats greedy on {:.1}% of seeds",
                rungs[top],
                beats(&results[top], &results[g]) * 100.0
            );
        }
    }
    println!("D: {depth} steps of {}", results.len() - 1);
    let top = results.last().unwrap();
    let mut picks: BTreeMap<Card, usize> = BTreeMap::new();
    for r in top {
        let mut seen: Vec<Card> = r.picks.clone();
        seen.sort();
        seen.dedup();
        for c in seen {
            *picks.entry(c).or_default() += 1;
        }
    }
    println!("\ncards the top rung took (share of runs):");
    let mut picks: Vec<_> = picks.into_iter().collect();
    picks.sort_by_key(|p| std::cmp::Reverse(p.1));
    for (card, n) in picks {
        println!(
            "  {:<11} {:>5.1}%",
            format!("{card:?}"),
            n as f64 / top.len() as f64 * 100.0
        );
    }
}
