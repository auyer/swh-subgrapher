# swh-subgrapher

`swh-subgrapher` is a Rust script designed to assist in the generation of Software Heritage Subgraphs. It takes a list of input Origins, retrieves the corresponding SWHIDs (Software Heritage Identifiers), and traverses the Software Heritage graph to find all associated objects.

This tool leverages the official [`swh-graph`](https://crates.io/crates/swh-graph) library to interact with the Software Heritage graph data.

The primary goal is to produce a list of SWHIDs that can then be used in the [Software Heritage `generate_subdataset` process](https://docs.softwareheritage.org/devel/swh-export/generate_subdataset.html) to create a custom, smaller dataset from the vast Software Heritage archive.

## Description

The script performs the following main functions:

1. **Loads Origins**: Reads a list of origin URLs from a specified input file.
2. **Loads Graph Data**: Initializes and loads the Software Heritage graph dataset from a local path.
3. **Resolves Origins to SWHIDs**: For each origin URL:
    * Calculates its SHA1 hash to form a potential SWHID.
    * Looks up the SWHID in the loaded graph.
    * If an origin is not found, and the `--try-protocol-variations` flag is set, it will attempt to find the origin by switching between `git://` and `https://` protocols.
4. **Graph Traversal**: For each successfully found origin node, it performs a Breadth-First Search (BFS) starting from that node to discover all reachable nodes (revisions, directories, contents, etc.) in the graph.
5. **Collects SWHIDs**: All unique SWHIDs encountered during the traversal are collected.
6. **Outputs Results**:
    * Writes the collected SWHIDs to `output.txt`, with each SWHID on a new line.
    * If any origins could not be found in the graph, their URLs are written to `errors.txt`.

## Prerequisites

* Rust programming language and Cargo (its package manager).
* A local copy of the Software Heritage graph dataset. You can find information on how to obtain this on the [Software Heritage documentation](https://docs.softwareheritage.org/devel/swh-export/graph/dataset.html#).
 	* the smaller “History and hosting” Compressed graph has everything needed for this task
* The `swh-graph` library and its dependencies must be available.

## Installation

1. Clone this repository or download the source code.
2. Navigate to the project directory.
3. Build the project using Cargo:

    ```bash
    cargo build --release
    ```

    The executable will be located in `target/release/swh-subgrapher`.

## Usage

To run the script, you need to provide the path to the Software Heritage graph dataset and the path to a file containing the list of origin URLs.

```bash
swh-subgrapher --graph /path/to/your/dataset/graph --origins origins.txt
