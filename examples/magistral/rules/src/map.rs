//! The map of a run: a river down the middle, hills, and the places that
//! make and want goods. The start city is on the west bank with a mine and a
//! farm in reach of plain track; the factory, the ore, the second city and a
//! second mine are across the river — reaching them is the run's big bet.

use std::collections::VecDeque;

use crate::{neighbours, Cell, Rng, Site, Terrain, CELLS, H, W};

pub fn generate(rng: &mut Rng) -> (Vec<Cell>, u8) {
    loop {
        if let Some(map) = attempt(rng) {
            return map;
        }
    }
}

fn blank() -> Cell {
    Cell {
        terrain: Terrain::Plain,
        site: None,
        track: false,
        station: false,
        stock: [0; 4],
        warehouse: false,
    }
}

fn at(x: usize, y: usize) -> usize {
    y * W + x
}

fn attempt(rng: &mut Rng) -> Option<(Vec<Cell>, u8)> {
    let mut cells = vec![blank(); CELLS];
    // The river: one or two cells wide where it bends, never broken.
    let mut x = 3 + rng.below(2);
    let mut river_x = [0usize; H];
    for (y, rx) in river_x.iter_mut().enumerate() {
        cells[at(x, y)].terrain = Terrain::Water;
        *rx = x;
        if rng.below(3) == 0 {
            let nx = if x == 3 { 4 } else { 3 };
            cells[at(nx, y)].terrain = Terrain::Water;
            x = nx;
        }
    }
    // Hills, in twos now and then.
    for _ in 0..4 {
        let c = rng.below(CELLS);
        if cells[c].terrain == Terrain::Plain {
            cells[c].terrain = Terrain::Hill;
            let n: Vec<u8> = neighbours(c as u8).collect();
            let m = n[rng.below(n.len())] as usize;
            if rng.below(2) == 0 && cells[m].terrain == Terrain::Plain {
                cells[m].terrain = Terrain::Hill;
            }
        }
    }
    let west = |c: usize| c % W < river_x[c / W].min(3);
    let east = |c: usize| c % W > river_x[c / W].max(4);
    let free = |cells: &Vec<Cell>, c: usize| {
        cells[c].terrain == Terrain::Plain
            && cells[c].site.is_none()
            && neighbours(c as u8).all(|n| cells[n as usize].site.is_none())
    };
    let mut place =
        |cells: &mut Vec<Cell>, site: Site, side: &dyn Fn(usize) -> bool| -> Option<usize> {
            for _ in 0..200 {
                let c = rng.below(CELLS);
                if side(c) && free(cells, c) {
                    cells[c].site = Some(site);
                    return Some(c);
                }
            }
            None
        };
    let start = place(&mut cells, Site::City, &|c| west(c) && c % W == 0)?;
    let coal = place(&mut cells, Site::Coal, &west)?;
    let farm = place(&mut cells, Site::Farm, &west)?;
    place(&mut cells, Site::City, &east)?;
    place(&mut cells, Site::Factory, &east)?;
    place(&mut cells, Site::Ore, &east)?;
    place(&mut cells, Site::Coal, &east)?;
    // The west bank must be playable on plain track alone.
    for goal in [coal, farm] {
        if !reachable(&cells, start, goal) {
            return None;
        }
    }
    cells[start].track = true;
    cells[start].station = true;
    Some((cells, start as u8))
}

/// Can plain track join `from` to `to`, around every other place?
fn reachable(cells: &[Cell], from: usize, to: usize) -> bool {
    let mut seen = vec![false; CELLS];
    let mut queue = VecDeque::from([from as u8]);
    seen[from] = true;
    while let Some(c) = queue.pop_front() {
        for n in neighbours(c) {
            if n as usize == to {
                return true;
            }
            let cell = &cells[n as usize];
            if !seen[n as usize] && cell.terrain == Terrain::Plain && cell.site.is_none() {
                seen[n as usize] = true;
                queue.push_back(n);
            }
        }
    }
    false
}
