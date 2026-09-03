use std::fs::{self, File, OpenOptions};
use std::hint::black_box;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant};

const CPU_ITERATIONS: u64 = 600_000_000;
const MEMORY_BYTES: usize = 512 * 1024 * 1024;
const MEMORY_PASSES: usize = 32;
const DISK_BYTES: usize = 512 * 1024 * 1024;
const FSYNC_WRITES: usize = 512;
const SAMPLES: usize = 3;

#[derive(Clone, Copy)]
struct CpuCounters {
    total: u64,
    steal: u64,
}

fn cpu_counters() -> Option<CpuCounters> {
    let stat = fs::read_to_string("/proc/stat").ok()?;
    let values: Vec<u64> = stat
        .lines()
        .next()?
        .split_whitespace()
        .skip(1)
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    Some(CpuCounters {
        total: values.iter().take(8).sum(),
        steal: values.get(7).copied().unwrap_or(0),
    })
}

fn steal_percent(before: Option<CpuCounters>, after: Option<CpuCounters>) -> f64 {
    match (before, after) {
        (Some(before), Some(after)) if after.total > before.total => {
            100.0 * (after.steal.saturating_sub(before.steal)) as f64
                / (after.total - before.total) as f64
        }
        _ => 0.0,
    }
}

fn timed<T>(f: impl FnOnce() -> T) -> (Duration, f64, T) {
    let counters_before = cpu_counters();
    let started = Instant::now();
    let value = f();
    let elapsed = started.elapsed();
    let steal = steal_percent(counters_before, cpu_counters());
    (elapsed, steal, value)
}

fn cpu_work(iterations: u64, seed: u64) -> u64 {
    let mut value = seed;
    for index in 0..iterations {
        value ^= index.wrapping_mul(0x9e37_79b9_7f4a_7c15);
        value = value.rotate_left(17).wrapping_mul(0xd6e8_feb8_6659_fd93);
        value ^= value >> 23;
        black_box(value);
    }
    value
}

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

fn cpu_benchmark(threads: usize) -> f64 {
    let mut throughputs = Vec::with_capacity(SAMPLES);
    for sample in 0..SAMPLES {
        let barrier = Arc::new(Barrier::new(threads + 1));
        let handles: Vec<_> = (0..threads)
            .map(|thread_index| {
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    cpu_work(CPU_ITERATIONS, (sample * threads + thread_index) as u64 + 1)
                })
            })
            .collect();
        let (elapsed, steal, checksum) = timed(|| {
            barrier.wait();
            handles
                .into_iter()
                .map(|handle| handle.join().expect("CPU worker panicked"))
                .fold(0, u64::wrapping_add)
        });
        black_box(checksum);
        let throughput = CPU_ITERATIONS as f64 * threads as f64 / elapsed.as_secs_f64();
        println!(
            "SAMPLE cpu_threads={threads} seconds={:.6} iterations_per_second={throughput:.0} steal_pct={steal:.3}",
            elapsed.as_secs_f64()
        );
        throughputs.push(throughput);
    }
    median(throughputs)
}

fn memory_benchmark() -> f64 {
    let source = vec![0x5au8; MEMORY_BYTES];
    let mut destination = vec![0u8; MEMORY_BYTES];
    let mut throughputs = Vec::with_capacity(SAMPLES);
    for sample in 0..SAMPLES {
        let (elapsed, steal, ()) = timed(|| {
            for pass in 0..MEMORY_PASSES {
                destination.copy_from_slice(&source);
                black_box(destination[(sample + pass) % MEMORY_BYTES]);
            }
        });
        let gib = (MEMORY_BYTES * MEMORY_PASSES) as f64 / 1024_f64.powi(3);
        let throughput = gib / elapsed.as_secs_f64();
        println!(
            "SAMPLE memory_copy seconds={:.6} gib_per_second={throughput:.3} steal_pct={steal:.3}",
            elapsed.as_secs_f64()
        );
        throughputs.push(throughput);
    }
    black_box(destination[MEMORY_BYTES - 1]);
    median(throughputs)
}

fn sequential_disk_benchmark(root: &Path) -> io::Result<f64> {
    let buffer = vec![0xa5u8; 1024 * 1024];
    let path = root.join("sequential.dat");
    let mut throughputs = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let (elapsed, steal, result) = timed(|| -> io::Result<()> {
            let mut file = File::create(&path)?;
            for _ in 0..(DISK_BYTES / buffer.len()) {
                file.write_all(&buffer)?;
            }
            file.sync_all()
        });
        result?;
        let throughput = DISK_BYTES as f64 / 1024_f64.powi(2) / elapsed.as_secs_f64();
        println!(
            "SAMPLE disk_sequential_sync seconds={:.6} mib_per_second={throughput:.3} steal_pct={steal:.3}",
            elapsed.as_secs_f64()
        );
        throughputs.push(throughput);
        fs::remove_file(&path)?;
    }
    Ok(median(throughputs))
}

fn fsync_benchmark(root: &Path) -> io::Result<f64> {
    let buffer = [0x3cu8; 4096];
    let path = root.join("fsync.dat");
    let mut latencies = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let (elapsed, steal, result) = timed(|| -> io::Result<()> {
            let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
            for _ in 0..FSYNC_WRITES {
                file.write_all(&buffer)?;
                file.sync_data()?;
            }
            Ok(())
        });
        result?;
        let micros = elapsed.as_secs_f64() * 1_000_000.0 / FSYNC_WRITES as f64;
        println!(
            "SAMPLE disk_fsync seconds={:.6} micros_per_fsync={micros:.3} steal_pct={steal:.3}",
            elapsed.as_secs_f64()
        );
        latencies.push(micros);
        fs::remove_file(&path)?;
    }
    Ok(median(latencies))
}

fn main() -> io::Result<()> {
    let scratch: PathBuf = std::env::args_os()
        .nth(1)
        .map(Into::into)
        .expect("usage: runner-perf-probe SCRATCH_DIRECTORY");
    println!(
        "FACT rust_available_parallelism={}",
        thread::available_parallelism()?.get()
    );

    let single_thread = cpu_benchmark(1);
    let four_threads = cpu_benchmark(4);
    let memory = memory_benchmark();
    let sequential_disk = sequential_disk_benchmark(&scratch)?;
    let fsync = fsync_benchmark(&scratch)?;

    println!("RESULT cpu_1t_iterations_per_second={single_thread:.0}");
    println!("RESULT cpu_4t_iterations_per_second={four_threads:.0}");
    println!("RESULT cpu_4t_scaling={:.3}", four_threads / single_thread);
    println!("RESULT memory_copy_gib_per_second={memory:.3}");
    println!("RESULT disk_sequential_sync_mib_per_second={sequential_disk:.3}");
    println!("RESULT disk_fsync_micros={fsync:.3}");
    Ok(())
}
