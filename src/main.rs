use argh::FromArgs;
use ethers::providers::{Middleware, Provider, ProviderError};
use futures::future::join_all;
use serde::Deserialize;
use std::collections::HashMap;
use tokio::io;
use tokio::io::AsyncBufReadExt;
use tokio::sync::broadcast;

fn default_count() -> usize {
    1_000
}

#[derive(Debug, FromArgs)]
/// Send the same query to multiple rpcs and compare responses
struct VersusConfig {
    #[argh(positional, greedy)]
    rpcs: Vec<String>,

    /// how many rpc calls to test
    /// TODO: make this optional. if not set, read all of them
    #[argh(option, default = "default_count()")]
    count: usize,
}

#[derive(Clone, Deserialize, Debug, PartialEq, Eq)]
#[serde(untagged)]
enum JsonRpcRequestEnum {
    Request(JsonRpcRequest),
    Batch(Vec<JsonRpcRequest>),
}

#[derive(Clone, Deserialize, Debug, PartialEq, Eq)]
struct JsonRpcRequest {
    method: String,
    params: Option<serde_json::Value>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config: VersusConfig = argh::from_env();

    let mut expected_chain_id = None;

    let mut http_providers = vec![];

    for rpc in config.rpcs.iter() {
        match Provider::try_from(rpc) {
            Ok(provider) => {
                let chain_id = provider.get_chainid().await?;

                println!("{}: chain_id {:#?}", rpc, chain_id);

                if let Some(expected_chain_id) = expected_chain_id {
                    if expected_chain_id != chain_id {
                        println!(
                            "{} has unexpected chain_id: {} != {}",
                            rpc, chain_id, expected_chain_id
                        );
                        continue;
                    }
                } else {
                    expected_chain_id = Some(chain_id);
                }

                http_providers.push(provider);
            }
            Err(err) => {
                println!("Failed connecting to {}: {:#?}", rpc, err);
                continue;
            }
        }
    }

    let num_providers = http_providers.len();

    if num_providers < 2 {
        println!("need at least 2 providers");
    }

    // TODO: how should we handle lagged? for now we just set a potentially very high capacity
    // TODO: i'd like to be able to send raw Strings so th
    let (tx, _) = broadcast::channel::<(usize, JsonRpcRequestEnum)>(config.count);

    let mut handles = Vec::with_capacity(num_providers);

    // TODO: i don't like this. we have to keep all results in memory until the end. change to wait for all of them and then do the processing
    for p in http_providers {
        let mut rx = tx.subscribe();

        let mut results = HashMap::with_capacity(config.count);

        let handle = tokio::spawn(async move {
            while let Ok((id, data)) = rx.recv().await {
                // println!("{} {}: {:?}", p.url(), i, data);

                match data {
                    JsonRpcRequestEnum::Request(request) => {
                        // TODO: spawn this so we can run in parallel
                        let response: Result<serde_json::Value, ProviderError> =
                            p.request(&request.method, &request.params).await;

                        results.insert(id, (request, response));
                    }
                    JsonRpcRequestEnum::Batch(batch) => {
                        for request in batch {
                            // TODO: spawn this so we can run in parallel
                            let response: Result<serde_json::Value, ProviderError> =
                                p.request(&request.method, &request.params).await;

                            results.insert(id, (request, response));
                        }
                    }
                }
            }

            (p, results)
        });

        handles.push(handle);
    }

    // read jsonrpc lines from stdin and send to all the providers
    let stdin = io::stdin();

    let reader = io::BufReader::new(stdin);

    let mut count = 0;
    let mut lines = reader.lines();

    while let Some(line) = lines.next_line().await? {
        match serde_json::from_str::<JsonRpcRequestEnum>(&line) {
            Ok(data) => {
                tx.send((count, data)).expect("unable to send");

                count += 1;
                if count >= config.count {
                    break;
                }
            }
            Err(err) => {
                println!("failed to parse jsonrpc request: {} {:#?}", line, err);
            }
        }
    }

    drop(tx);

    println!("sent {}/{} requests", count, config.count);

    let mut map = HashMap::with_capacity(count);
    let mut providers = Vec::with_capacity(num_providers);

    for res in join_all(handles).await {
        match res {
            Ok((p, results)) => {
                map.insert(p.url().to_string(), results);
                providers.push(p);
            }
            Err(err) => println!("join error! {:#?}", err),
        };
    }

    let first_provider = providers[0].url().to_string();

    let first_map = map.get(&first_provider).unwrap();

    let mut mismatched = 0;

    // TODO: this is super slow. refactor
    for i in 0..count {
        if let Some((first_request, first_response)) = first_map.get(&i) {
            for p in providers.iter().skip(1) {
                let url = p.url().as_str();

                let compare_map = map.get(url).unwrap();

                if let Some((request, response)) = compare_map.get(&i) {
                    if first_request != request {
                        panic!("request mismatch");
                    }

                    match (first_response, response) {
                        (Ok(compare), Ok(first)) => {
                            if compare != first {
                                println!("{} {}: {} != {}", url, i, compare, first);
                                mismatched += 1;
                            }
                        }
                        (Ok(compare), Err(first)) => {
                            println!("{} {}: {} != {:?}", url, i, compare, first);
                            mismatched += 1;
                        }
                        (Err(compare), Ok(first)) => {
                            println!("{} {}: {:?} != {}", url, i, compare, first);
                            mismatched += 1;
                        }
                        (Err(compare), Err(first)) => {
                            let compare = format!("{:?}", compare);
                            let first = format!("{:?}", first);

                            if compare != first {
                                println!("{} {}: {} != {}", url, i, compare, first);
                                mismatched += 1;
                            }
                        }
                    }
                } else {
                    println!("howd this happen to the compare provider?");
                    mismatched += 1;
                }
            }
        } else {
            println!("howd this happen to the first provider?");
            mismatched += 1;
        }
    }

    if mismatched > 0 {
        return Err(anyhow::anyhow!("mismatched results!"));
    }

    println!("all matched! yey!");

    Ok(())
}
