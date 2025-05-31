// Copyright (C) 2025  The Software Heritage developers
// See the AUTHORS file at the top-level directory of this distribution
// License: GNU General Public License version 3, or any later version
// See top-level LICENSE file for more information

use swh_graph::SWHID;

use std::collections::{HashSet, VecDeque};
use std::fmt::Display;
use std::fs::File;
use std::io::{self, prelude::*, BufReader, BufWriter, Lines};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::Parser;
use dsi_progress_logger::{progress_logger, ProgressLog};
use log::{debug, error, info, warn, Level};

use swh_graph::collections::{AdaptiveNodeSet, NodeSet};
use swh_graph::graph::SwhGraphWithProperties;
use swh_graph::graph::{self, SwhForwardGraph};
use swh_graph::mph::DynMphf;
use swh_graph::properties;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// path location to the base graph. It should contain prefixes if they are present in the file
    /// names. Check the docs for more details
    #[arg(short, long)]
    graph: PathBuf,
    /// path to a file with a list of origins to be searched.
    /// Origins should be one by line, without any extra chars
    #[arg(short, long)]
    origins: PathBuf,
    /// in case an origin is not found in the graph, this allows the script to attempt to find it
    /// with another protocol (https:// <-> git://)
    #[arg(short = 'p', long, default_value_t = false)]
    allow_protocol_variations: bool,
    /// path to folder or file name for the output. If any origin is not found in the graph,
    /// a file named `origin_errors.txt` will be written in the same path
    #[arg(short = 'O', long)]
    output: PathBuf,
}

pub fn main() -> Result<()> {
    let args = Args::parse();

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    debug!("Debug logging ON...");

    info!("Loading origins...");
    let origins_lines = lines_from_file(args.origins).expect("Unable to read origins file");

    info!("Loading graph...");
    let graph = graph::SwhUnidirectionalGraph::new(args.graph)
        .context("Could not load graph")?
        .init_properties()
        .load_properties(|properties| properties.load_maps::<DynMphf>())
        .context("Could not load graph properties")?;

    let (subgraph_nodes, unknown_origins) =
        process_origins_and_build_subgraph(&graph, origins_lines, args.allow_protocol_variations);

    debug!(
        "Writing list of nodes to '{}'...",
        args.output.as_path().display()
    );

    // Call the function and handle the result
    match write_items_to_file(
        subgraph_nodes
            .iter()
            // convert NodeID into SWHID
            .map(|node| graph.properties().swhid(*node)),
        args.output.clone(),
    ) {
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
        let errors_filename = args.output.with_file_name("origin_errors.txt");

        warn!(
            "Some of the requested origins could not be found in the graph. Writing failed origins to '{}'...",
            errors_filename.as_path().display()
        );

        // Call the function and handle the result
        write_items_to_file(&unknown_origins, errors_filename)?;
    }

    Ok(())
}

fn process_origins_and_build_subgraph<G, I>(
    graph: &G,
    origins: I,
    allow_protocol_variations: bool,
) -> (HashSet<usize>, Vec<String>)
where
    G: SwhGraphWithProperties + SwhForwardGraph,
    G::Maps: properties::Maps,
    I: Iterator<Item = Result<String, std::io::Error>>,
{
    let graph_props = graph.properties();
    let num_nodes = graph.num_nodes();

    let mut subgraph_nodes = HashSet::new();
    let mut unknown_origins = vec![];

    let mut pl = progress_logger!(
        display_memory = true,
        item_name = "node",
        local_speed = true,
        expected_updates = Some(num_nodes),
    );
    pl.start("visiting graph ...");

    for origin_result in origins {
        if origin_result.is_err() {
            let err = origin_result.err().unwrap();
            error!("failed reading line from origins file: {err}");
            continue;
        }
        let origin = origin_result.unwrap();
        let mut origin_swhid = SWHID::from_origin_url(origin.to_owned());

        // Lookup SWHID
        info!("looking up SWHID {} ...", origin);
        let mut node_id_lookup = graph_props.node_id(origin_swhid);

        if node_id_lookup.is_err() && allow_protocol_variations {
            warn!("origin {origin} not in graph. Will look for other protocols");
            // try with other protocols
            if origin.contains("git://") || origin.contains("https://") {
                // try to switch the protocol. Only https and git available
                let alternative_origin = if origin.contains("git://") {
                    origin.replace("git://", "https://")
                } else if origin.contains("https://") {
                    origin.replace("https://", "git://")
                } else {
                    origin.to_owned()
                };

                origin_swhid = SWHID::from_origin_url(alternative_origin.to_owned());

                node_id_lookup = graph_props.node_id(origin_swhid);
                if node_id_lookup.is_ok() {
                    debug!("origin found with different protocol: {origin}");
                }
            }
        }

        // if node_id is still err, attempts to switch protocols failed
        // the original url from the origins file should be logged
        let Ok(node_id) = node_id_lookup else {
            error!("origin {origin} not in graph");
            unknown_origins.push(origin);
            continue;
        };
        debug!("obtained node ID {node_id} ...");
        assert!(node_id < num_nodes);

        // Setup a queue and a visited AdaptiveNodeSet for the visits
        let mut visited = AdaptiveNodeSet::new(num_nodes);
        let mut queue: VecDeque<usize> = VecDeque::new();

        queue.push_back(node_id);

        // Setup the progress logger for
        let mut visited_nodes = 0;

        debug!("starting bfs for the origin: {origin}");

        // iterative BFS
        while let Some(current_node) = queue.pop_front() {
            if log::log_enabled!(Level::Debug) {
                let id = graph.properties().swhid(current_node);
                debug!("visited: {id}");
            } // add current_node to the external results hashset
            let new = subgraph_nodes.insert(current_node);
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
            } else if log::log_enabled!(Level::Debug) {
                debug!(
                    "stopping bfs because this node was foud in a previous bfs run (from another origin) {current_node}"
                );
            }
        }

        if log::log_enabled!(Level::Info) {
            pl.update_and_display();
        }
        info!("visit from {origin} completed after visiting {visited_nodes} nodes.");
    }
    pl.done();

    (subgraph_nodes, unknown_origins)
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

fn lines_from_file(filename: impl AsRef<Path>) -> io::Result<Lines<BufReader<File>>> {
    let file = File::open(filename)?;
    let reader = BufReader::new(file);
    // returns the iterator from BufReader::lines()
    Ok(reader.lines())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::iter;
    // use swh_graph::labels::EdgeLabel::Branch;
    use swh_graph::graph_builder::GraphBuilder;
    // use swh_graph::labels::Permission;
    use swh_graph::swhid;

    use swh_graph::labels::VisitStatus;

    // this graph is a based of the one found in
    // rust/tests/test_label_iterators.rs:build_graph
    //
    //
    ///// ```
    /// ori0 -->  snp2 -->  rev4
    ///          ^    \
    ///         /      \
    ///        /        \
    /// ori1 -+          -> rev5 -> rev6
    ///     \           /
    ///      \         /
    ///       \       /
    ///        -> snp3
    ///
    /// ori7 --> rev8
    ///  (disjoint graph)
    /// ```
    #[test]
    fn test_process_origins_and_build_subgraph() {
        let mut builder = GraphBuilder::default();

        builder
            .node(SWHID::from_origin_url(
                "https://example.com/repo1".to_owned(),
            ))
            .unwrap()
            .done();
        builder
            .node(SWHID::from_origin_url(
                "https://example.com/repo2".to_owned(),
            ))
            .unwrap()
            .done();
        builder
            .node(swhid!(swh:1:snp:0000000000000000000000000000000000000002))
            .unwrap()
            .done();
        builder
            .node(swhid!(swh:1:snp:0000000000000000000000000000000000000003))
            .unwrap()
            .done();
        builder
            .node(swhid!(swh:1:rev:0000000000000000000000000000000000000004))
            .unwrap()
            .done();
        builder
            .node(swhid!(swh:1:rev:0000000000000000000000000000000000000005))
            .unwrap()
            .done();
        builder
            .node(swhid!(swh:1:rev:0000000000000000000000000000000000000006))
            .unwrap()
            .done();

        builder.ori_arc(0, 2, VisitStatus::Full, 1000002000);
        builder.ori_arc(0, 2, VisitStatus::Full, 1000002001);
        builder.ori_arc(0, 3, VisitStatus::Full, 1000003000);
        builder.ori_arc(1, 2, VisitStatus::Full, 1001002000);
        builder.snp_arc(2, 4, b"refs/heads/snp2-to-rev4");
        builder.snp_arc(2, 5, b"refs/heads/snp2-to-rev5");
        builder.snp_arc(3, 5, b"refs/heads/snp3-to-rev5");
        builder.snp_arc(3, 5, b"refs/heads/snp3-to-rev5-dupe");
        builder.ori_arc(5, 6, VisitStatus::Full, 1001006000);

        // disjoint graph
        builder
            .node(SWHID::from_origin_url(
                "https://example.com/discinnected".to_owned(),
            ))
            .unwrap()
            .done();
        builder
            .node(swhid!(swh:1:rev:0000000000000000000000000000000000000008))
            .unwrap()
            .done();

        builder.ori_arc(7, 8, VisitStatus::Full, 1001007000);

        let graph = builder.done().unwrap();

        let origins = vec![
            Ok("https://example.com/repo1".to_string()),
            // this one should be found with allow_protocol_variations
            Ok("git://example.com/repo2".to_string()),
            Ok("https://unknown.com/repo".to_string()),
        ];
        let (subgraph_nodes, unknown_origins) =
            process_origins_and_build_subgraph(&graph, origins.into_iter(), true);

        // Check that we found the expected nodes
        assert_eq!(subgraph_nodes.len(), 7); // should contain both origins and the revision
        assert_eq!(unknown_origins.len(), 1); // the unknown origin
        assert_eq!(unknown_origins[0], "https://unknown.com/repo");

        // Test with empty input
        let (empty_nodes, empty_unknown) =
            process_origins_and_build_subgraph(&graph, iter::empty(), false);
        assert!(empty_nodes.is_empty());
        assert!(empty_unknown.is_empty());

        // Test with only invalid origins
        let invalid_origins = vec![
            Ok("https://invalid1.com".to_string()),
            Ok("https://invalid2.com".to_string()),
        ];
        let (invalid_nodes, invalid_unknown) =
            process_origins_and_build_subgraph(&graph, invalid_origins.into_iter(), false);
        assert!(invalid_nodes.is_empty());
        assert_eq!(invalid_unknown.len(), 2);

        // Test with only invalid origins
        let disjoint_origins = vec![Ok("https://example.com/discinnected".to_string())];
        let (disjoint_nodes, disjoint_unknown) =
            process_origins_and_build_subgraph(&graph, disjoint_origins.into_iter(), false);
        assert!(disjoint_unknown.is_empty());
        assert_eq!(disjoint_nodes.len(), 2);
    }
}
