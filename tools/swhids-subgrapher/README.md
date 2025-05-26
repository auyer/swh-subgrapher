# swhids-subgrapher

`swhids-subgrapher` is a Rust script designed to assist in the generation of Software Heritage Subgraphs.
It takes a list of input Origins, retrieves the corresponding SWHIDs (Software Heritage Identifiers), and traverses the Software Heritage graph to find all associated objects.

This tool leverages the official [`swh-graph`](https://crates.io/crates/swh-graph) library to interact with the Software Heritage graph data.

The primary goal is to produce a list of SWHIDs that can then be used in the [Software Heritage `generate_subdataset` process](https://docs.softwareheritage.org/devel/swh-export/generate_subdataset.html) to create a custom, smaller dataset from the vast Software Heritage archive.

## Description

The script performs the following main functions:

1. **Loads Origins**: Reads a list of origin URLs from a specified input file.
2. **Loads Graph Data**: Initializes and loads the Software Heritage graph dataset from a path.
3. **Resolves Origins to SWHIDs**: For each origin URL:
    * Calculates its SHA1 hash to form a potential SWHID.
    * Looks up the SWHID in the loaded graph.
    * If an origin is not found, and the `--try-protocol-variations / --t` flag is set, it will attempt to find the origin by switching between `git://` and `https://` protocols.
4. **Graph Traversal**: For each successfully found origin node, it performs a Breadth-First Search (BFS) starting from that node to discover all reachable nodes in the graph.
5. **Collects SWHIDs**: All unique SWHIDs encountered during the traversal are collected.
6. **Outputs Results**:
    * Writes the collected SWHIDs to a file (`--output` flag), with each SWHID on a new line.
    * If a origin could not be found in the graph, its URL will be written to a  written to a `origin_errors.txt` file in the same path of the output.

## Prerequisites

* Rust programming language and Cargo (its package manager).
* A local copy of a Software Heritage graph dataset. You can find information on how to obtain this on the [Software Heritage documentation](https://docs.softwareheritage.org/devel/swh-export/graph/dataset.html#).
  * The result will be a sugraph of the one used as input.
* This uses the `swh-graph` library, and it requires some system dependencies not provided by cargo. Check the [swh-graph quickstart](https://docs.softwareheritage.org/devel/swh-graph/quickstart.html) docs for more information.

## Installation

1. Clone this repository or download the source code.
2. Navigate to the project directory.
3. Build the project using Cargo:

    ```bash
    cargo build --release
    ```

    The executable will be located in `target/release/swhids-subgrapher`.

## Usage

To run the script, you need to provide the path to the Software Heritage graph dataset and the path to a file containing the list of origin URLs.

```bash
swhids-subgrapher --graph /path/to/dataset/2024-12-06-history-hosting/graph -t --origins origins.txt --output results
```

> ⚠️ The Software Heritage dataset files usually have a "graph" prefix.
> The `swh-graph` library, used to read the graph, expects the path to contain the prefix common to all files.
>
> Example: `graph.graph`, `graph-labelled.ef`, `graph-transposed.properties` are some of the files that should be in the dataset path, and have a "graph" prefix in the name.
> This is the reason the example above has a `/graph` at the end of the path.

### Debugging

If facing issues, try running with DEBUG logs to get a more detailed view of what is happening:

```bash
RUST_LOG=debug swhids-subgrapher ...
```
