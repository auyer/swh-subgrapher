use sha1::{Digest, Sha1};

use swh_graph::graph::SwhGraphWithProperties;
use swh_graph::graph::{self, SwhForwardGraph, SwhGraph};
use swh_graph::mph::DynMphf;

use anyhow::{Context, Result};
use bitvec::prelude::*;
use clap::Parser;
use dsi_progress_logger::{ProgressLog, progress_logger};
use log::{debug, error, info};
use std::{
    collections::{HashSet, VecDeque},
    fmt::Display,
    fs::File,
    io::{self, BufReader, BufWriter, prelude::*},
    path::{Path, PathBuf},
};

#[derive(Parser, Debug)]
#[command(about, long_about = None)]
struct Args {
    #[arg(short, long)]
    graph: PathBuf,
    #[arg(short, long)]
    origins: PathBuf,
    #[arg(short, long)]
    #[clap(default_value_t = false)]
    try_protocol_variations: bool,
}

pub fn main() -> Result<()> {
    let args = Args::parse();

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    info!("Loading origins...");
    let origins = lines_from_file(args.origins);

    info!("Loading graph...");
    let graph = graph::SwhUnidirectionalGraph::new(args.graph)
        .context("Could not load graph")?
        .init_properties()
        .load_properties(|properties| properties.load_maps::<DynMphf>())
        .context("Could not load graph properties")?;

    let graph_props = graph.properties();

    let mut subgraph_nodes = HashSet::new();

    let mut unknown_origins = vec![];

    for origin in origins.iter() {
        let origin_hash = get_sha1(origin.to_owned());

        // "swh:1:ori:{}"
        let origin_swhid = swh_graph::SWHID {
            namespace_version: 1,
            node_type: swh_graph::NodeType::Origin,
            hash: origin_hash,
        };

        // Lookup SWHID
        info!("looking up SWHID {} ...", origin);
        let mut node_id = graph_props.node_id(origin_swhid).context("Unknown SWHID");

        if node_id.is_err() && args.try_protocol_variations {
            error!("origin {origin} not in graph. Will look for other protocols");
            // try with other protocols
            if origin.contains("git://") || origin.contains("https://") {
                let new_origin = if origin.contains("git://") {
                    origin.replace("git://", "https://")
                } else if origin.contains("https://") {
                    origin.replace("https://", "git://")
                } else {
                    origin.to_owned()
                };

                let origin_hash = get_sha1(new_origin.to_owned());

                let origin_swhid = swh_graph::SWHID {
                    namespace_version: 1,
                    node_type: swh_graph::NodeType::Origin,
                    hash: origin_hash,
                };

                node_id = graph_props.node_id(origin_swhid).context("Unknown SWHID");
                if node_id.is_ok() {
                    debug!("origin found with different protocol {origin} -> {new_origin}");
                }
            }
        }

        // if node_id is still err, attempts to switch protocols failed
        if node_id.is_err() {
            error!("origin {origin} not in graph");
            unknown_origins.push(origin);
            continue;
        }
        let node_id = node_id.unwrap();
        info!("obtained node ID {node_id} ...");

        // Setup a queue and a visited bitmap for the visit
        let num_nodes = graph.num_nodes();
        let mut visited = bitvec![u64, Lsb0; 0; num_nodes];
        let mut queue: VecDeque<usize> = VecDeque::new();
        assert!(node_id < num_nodes);
        queue.push_back(node_id);

        // Setup the progress logger for
        let mut visited_nodes = 0;
        let mut pl = progress_logger!(
            display_memory = true,
            item_name = "node",
            local_speed = true,
            expected_updates = Some(num_nodes),
        );
        pl.start("visiting graph ...");

        // Standard BFS
        while let Some(current_node) = queue.pop_front() {
            let visited_swhid = graph.properties().swhid(current_node);
            debug!("{visited_swhid}");
            // add current_node to the external results hashmap
            let new = subgraph_nodes.insert(visited_swhid.to_string());
            //  only visit children if this node is new
            if new {
                visited_nodes += 1;
                for succ in graph.successors(current_node) {
                    if !visited[succ] {
                        queue.push_back(succ);
                        visited.set(succ as _, true);
                        pl.light_update();
                    }
                }
            }
        }

        pl.done();
        info!("visit completed after visiting {visited_nodes} nodes.");
    }

    let output_filename = "output.txt";

    println!("Attempting to write HashSet to '{}'...", output_filename);

    // Call the function and handle the result
    match write_items_to_file(&subgraph_nodes, output_filename) {
        Ok(_) => println!("Successfully wrote HashSet to '{}'.", output_filename),
        Err(e) => eprintln!("Error writing to file '{}': {}", output_filename, e),
    }

    // if there are origins that failed to be found
    if !unknown_origins.is_empty() {
        let errors_filename = "errors.txt";

        println!(
            "Some of the requested origins could not be found in the graph. \nAttempting to write failed origins to '{}'...",
            errors_filename
        );

        // Call the function and handle the result
        write_items_to_file(&unknown_origins, errors_filename)?;
    }

    Ok(())
}

// write_items_to_file can take hanshmaps and vecs
fn write_items_to_file<P, I>(items: I, filename: P) -> io::Result<()>
where
    P: AsRef<Path>, // Accept anything convertible to a Path reference (like &str, String, PathBuf)
    I: IntoIterator, // The input must be iterable
    I::Item: Display, // The items produced by the iterator must implement Display
{
    let file = File::create(filename)?;

    // Wrap the file in a BufWriter for better performance.
    // Writing directly to a file can be slow due to many small system calls.
    // BufWriter collects writes in a buffer and flushes them in larger chunks.
    let mut writer = BufWriter::new(file);
    // Iterate over the elements (strings) in the HashSet.
    for item in items {
        writeln!(writer, "{}", item)?;
    }

    Ok(())
}

fn get_sha1(origin: String) -> [u8; 20] {
    // create a Sha1 object
    let mut hasher = Sha1::new();

    // process input message
    hasher.update(origin);

    // acquire hash digest in the form of GenericArray,
    // which in this case is equivalent to [u8; 20]
    let result = hasher.finalize();
    result.into()
}

fn lines_from_file(filename: impl AsRef<Path>) -> Vec<String> {
    let file = File::open(filename).expect("no such file");
    let buf = BufReader::new(file);
    buf.lines()
        .map(|l| l.expect("Could not parse line"))
        .collect()
}
