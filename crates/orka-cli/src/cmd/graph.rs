use crate::client::OrkaClient;
use crate::table::make_table;
use colored::Colorize;

pub async fn show(client: &OrkaClient, dot: bool) -> crate::client::Result<()> {
    let body = client.get_json("/api/v1/graph").await?;

    if dot {
        print_dot(&body);
        return Ok(());
    }

    let id = body["id"].as_str().unwrap_or("?");
    let entry = body["entry"].as_str().unwrap_or("?");
    println!("{} {}", "Graph:".cyan(), id.bold());
    println!("Entry: {}", entry.green());

    if let Some(termination) = body["termination"].as_object() {
        let max_iter = termination["max_total_iterations"].as_u64().unwrap_or(0);
        let max_dur = termination["max_duration_secs"].as_u64().unwrap_or(0);
        println!("Termination: max_iterations={max_iter} max_duration={max_dur}s");
    }

    let empty = serde_json::Value::Array(vec![]);
    let nodes = body["nodes"]
        .as_array()
        .unwrap_or(empty.as_array().unwrap());
    if !nodes.is_empty() {
        println!();
        let mut table = make_table(&["ID", "Kind", "Agent", "Max Iter"]);
        for node in nodes {
            let max_iter = node["agent"]["max_iterations"]
                .as_u64()
                .unwrap_or(0)
                .to_string();
            table.add_row([
                node["id"].as_str().unwrap_or("?"),
                node["kind"].as_str().unwrap_or("?"),
                node["agent"]["name"].as_str().unwrap_or("?"),
                &max_iter,
            ]);
        }
        println!("{table}");
    }

    let edges = body["edges"]
        .as_array()
        .unwrap_or(empty.as_array().unwrap());
    if !edges.is_empty() {
        println!();
        let mut table = make_table(&["From", "To", "Priority", "Condition"]);
        for edge in edges {
            let prio = edge["priority"].as_u64().unwrap_or(0).to_string();
            let cond_owned = serde_json::to_string(&edge["condition"]).unwrap_or_default();
            let cond = edge["condition"].as_str().unwrap_or(&cond_owned);
            table.add_row([
                edge["from"].as_str().unwrap_or("?"),
                edge["to"].as_str().unwrap_or("?"),
                &prio,
                cond,
            ]);
        }
        println!("{table}");
    }

    Ok(())
}

fn print_dot(body: &serde_json::Value) {
    println!("digraph orka {{");
    println!("  rankdir=LR;");
    println!("  node [shape=box, style=filled, fillcolor=lightblue];");

    let entry = body["entry"].as_str().unwrap_or("?");
    println!("  \"{entry}\" [fillcolor=lightgreen];");

    let empty_nodes = serde_json::Value::Array(vec![]);
    let nodes = body["nodes"]
        .as_array()
        .unwrap_or(empty_nodes.as_array().unwrap());
    for node in nodes {
        let nid = node["id"].as_str().unwrap_or("?");
        let name = node["agent"]["name"].as_str().unwrap_or("?");
        let kind = node["kind"].as_str().unwrap_or("Agent");
        println!("  \"{nid}\" [label=\"{name}\\n({kind})\"];");
    }

    let empty_edges = serde_json::Value::Array(vec![]);
    let edges = body["edges"]
        .as_array()
        .unwrap_or(empty_edges.as_array().unwrap());
    for edge in edges {
        let from = edge["from"].as_str().unwrap_or("?");
        let to = edge["to"].as_str().unwrap_or("?");
        let cond = edge["condition"].as_str().unwrap_or("always");
        println!("  \"{from}\" -> \"{to}\" [label=\"{cond}\"];");
    }

    println!("}}");
}
