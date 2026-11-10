// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Run two to four linked cores headlessly and report speed and link state.
//!
//! `cargo run --release -p mgba-core --example link -- game.gba [other.gba ...] [--players N] [--frames N]`
//!
//! With one ROM, `--players` copies of it are linked (default 2). Game Boy
//! ROMs link two at a time.

use mgba_core::gb::GbSioLockstep;
use mgba_core::gba::GbaSioLockstepCoordinator;
use mgba_core::lockstep::run_linked_frame;
use mgba_core::{gb, Gb, Gba};

fn main() {
    let mut roms = vec![];
    let mut players = 2;
    let mut frames = 600;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--players" => players = args.next().and_then(|v| v.parse().ok()).expect("--players N"),
            "--frames" => frames = args.next().and_then(|v| v.parse().ok()).expect("--frames N"),
            _ => roms.push(a),
        }
    }
    if roms.is_empty() {
        eprintln!("usage: link ROM [ROM...] [--players N] [--frames N]");
        std::process::exit(2);
    }
    if roms.len() == 1 {
        roms = vec![roms[0].clone(); players];
    }
    let data: Vec<Vec<u8>> = roms.iter().map(|r| std::fs::read(r).expect("read ROM")).collect();
    let start = std::time::Instant::now();
    let deadlocks;
    if gb::is_rom(&data[0]) {
        let link = GbSioLockstep::new();
        let mut cores: Vec<Box<Gb>> = data
            .into_iter()
            .take(2)
            .map(|d| {
                let mut g = Box::new(Gb::new());
                g.load_rom(d);
                g.reset();
                assert!(g.attach_link(&link));
                g
            })
            .collect();
        let mut stalls = 0;
        for _ in 0..frames {
            let mut refs: Vec<&mut Gb> = cores.iter_mut().map(|c| c.as_mut()).collect();
            stalls += !run_linked_frame(&mut refs) as u32;
        }
        deadlocks = stalls;
        for (i, c) in cores.iter().enumerate() {
            println!("player {}: node {} frame {}", i + 1, c.link().unwrap().id(), c.frame_counter());
        }
    } else {
        let link = GbaSioLockstepCoordinator::new();
        let mut cores: Vec<Box<Gba>> = data
            .into_iter()
            .enumerate()
            .map(|(i, d)| {
                let mut g = Box::new(Gba::new());
                g.load_rom(d);
                g.reset();
                g.attach_link(&link, Some(i as i32));
                g
            })
            .collect();
        let mut stalls = 0;
        for _ in 0..frames {
            let mut refs: Vec<&mut Gba> = cores.iter_mut().map(|c| c.as_mut()).collect();
            stalls += !run_linked_frame(&mut refs) as u32;
        }
        deadlocks = stalls;
        for (i, c) in cores.iter_mut().enumerate() {
            println!(
                "player {}: id {} frame {} SIOCNT {:04X} RCNT {:04X} mode {}",
                i + 1,
                c.link_player_id(),
                c.frame_counter(),
                c.sio.siocnt,
                c.sio.rcnt,
                c.sio.mode
            );
        }
    }
    let secs = start.elapsed().as_secs_f64();
    println!("{frames} linked frames in {secs:.2} s ({:.1} fps), {deadlocks} stalled", frames as f64 / secs);
}
