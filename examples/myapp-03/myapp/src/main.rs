use aya::{include_bytes_aligned, Bpf};
use anyhow::Context;
use aya::programs::{Xdp, XdpFlags};
use aya::maps::{perf::AsyncPerfEventArray, HashMap};
use aya::util::online_cpus;
use bytes::BytesMut;
use std::{env, net::{self, Ipv4Addr}};
use tokio::{signal, task};

use myapp_common::PacketLog;

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let iface = match env::args().nth(1) {
        Some(iface) => iface,
        None => "eth0".to_string(),
    };

    #[cfg(debug_assertions)]
    let mut bpf = Bpf::load(include_bytes_aligned!(
        "../../target/bpfel-unknown-none/debug/myapp"
    ))?;
    #[cfg(not(debug_assertions))]
    let mut bpf = Bpf::load(include_bytes_aligned!(
        "../../target/bpfel-unknown-none/release/myapp"
    ))?;
    let probe: &mut Xdp = bpf.program_mut("xdp").unwrap().try_into()?;
    probe.load()?;
    probe.attach(&iface, XdpFlags::default())
        .context("failed to attach the XDP program with default flags - try changing XdpFlags::default() to XdpFlags::SKB_MODE")?;

    // (1)
    let mut blocklist: HashMap<_, u32, u32> =
        HashMap::try_from(bpf.map_mut("BLOCKLIST")?)?;

    // (2)
    let block_addr: u32 = Ipv4Addr::new(1, 1, 1, 1).try_into()?;

    // (3)
    blocklist.insert(block_addr, 0, 0)?;

    let mut perf_array = AsyncPerfEventArray::try_from(bpf.map_mut("EVENTS")?)?;

    for cpu_id in online_cpus()? {
        let mut buf = perf_array.open(cpu_id, None)?;

        task::spawn(async move {
            let mut buffers = (0..10)
                .map(|_| BytesMut::with_capacity(1024))
                .collect::<Vec<_>>();

            loop {
                let events = buf.read_events(&mut buffers).await.unwrap();
                for i in 0..events.read {
                    let buf = &mut buffers[i];
                    let ptr = buf.as_ptr() as *const PacketLog;
                    let data = unsafe { ptr.read_unaligned() };
                    let src_addr = net::Ipv4Addr::from(data.ipv4_address);
                    println!("LOG: SRC {}, ACTION {}", src_addr, data.action);
                }
            }
        });
    }
    signal::ctrl_c().await.expect("failed to listen for event");
    Ok::<_, anyhow::Error>(())
}
