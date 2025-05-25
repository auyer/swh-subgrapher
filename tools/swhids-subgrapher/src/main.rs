// Copyright (C) 2025  The Software Heritage developers
// See the AUTHORS file at the top-level directory of this distribution
// License: GNU General Public License version 3, or any later version
// See top-level LICENSE file for more information

use swh_graph::SWHID;

use std::collections::{HashSet, VecDeque};
use std::fmt::Display;
use std::fs::File;
use std::io::{self, BufReader, BufWriter, prelude::*};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::Parser;
use dsi_progress_logger::{ProgressLog, progress_logger};
use log::{debug, error, info, warn};

use swh_graph::collections::{AdaptiveNodeSet, NodeSet};
use swh_graph::graph::SwhGraphWithProperties;
use swh_graph::graph::{self, SwhForwardGraph, SwhGraph};
use swh_graph::mph::DynMphf;

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
    #[arg(long)]
    output: PathBuf,
}

pub fn main() -> Result<()> {
    let args = Args::parse();

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    info!("Loading origins...");
    let origins = lines_from_file(args.origins).expect("Unable to read origins file");

    info!("Loading graph...");
    let graph = graph::SwhUnidirectionalGraph::new(args.graph)
        .context("Could not load graph")?
        .init_properties()
        .load_properties(|properties| properties.load_maps::<DynMphf>())
        .context("Could not load graph properties")?;

    let graph_props = graph.properties();
    let num_nodes = graph.num_nodes();

    let mut subgraph_nodes = HashSet::new();

    let mut unknown_origins = vec![];

    for origin in origins.iter() {
        let origin_swhid = SWHID::from_origin_url(origin.to_owned());

        // Lookup SWHID
        info!("looking up SWHID {} ...", origin);
        let mut node_id = graph_props.node_id(origin_swhid);

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

                let origin_swhid = SWHID::from_origin_url(new_origin.to_owned());

                node_id = graph_props.node_id(origin_swhid);
                if node_id.is_ok() {
                    debug!("origin found with different protocol {origin} -> {new_origin}");
                }
            }
        }

        // if node_id is still err, attempts to switch protocols failed
        let Ok(node_id) = node_id else {
            error!("origin {origin} not in graph");
            unknown_origins.push(origin);
            continue;
        };
        info!("obtained node ID {node_id} ...");

        // Setup a queue and a visited AdaptiveNodeSet for the visits
        let mut visited = AdaptiveNodeSet::new(num_nodes);
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
            debug!("visited: {visited_swhid}");
            // add current_node to the external results hashmap
            let new = subgraph_nodes.insert(visited_swhid);
            //  only visit children if this node is new
            if new {
                visited_nodes += 1;
                for succ in graph.successors(current_node) {
                    if !visited.contains(succ) {
                        queue.push_back(succ);
                        visited.insert(succ);
                        pl.light_update();
                    }
                }
            }
        }

        pl.done();
        info!("visit completed after visiting {visited_nodes} nodes.");
    }

    debug!(
        "Writing list of nodes to '{}'...",
        args.output.as_path().display()
    );

    // Call the function and handle the result
    match write_items_to_file(&subgraph_nodes, args.output.clone()) {
        Ok(_) => info!(
            "Successfully wrote list of nodes to '{}'.",
            args.output.as_path().display()
        ),
        Err(e) => error!(
            "Error writing to file '{}': {}",
            args.output.as_path().display(),
            e
        ),
    }

    // if there are origins that failed to be found
    if !unknown_origins.is_empty() {
        let mut errors_filename = args.output;
        errors_filename.push("_errors");

        warn!(
            "Some of the requested origins could not be found in the graph.\nWriting failed origins to '{}'...",
            errors_filename.as_path().display()
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

fn lines_from_file(filename: impl AsRef<Path>) -> Result<Vec<String>> {
    let file = File::open(filename)?;
    let buf = BufReader::new(file);
    Ok(buf
        .lines()
        .map(|l| l.expect("Could not parse line"))
        .collect())
}
