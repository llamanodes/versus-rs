use argh::FromArgs;
use counter::Counter;
use futures::future::join_all;
use reqwest::Url;
use std::sync::atomic::AtomicUsize;
use tokio::io;
use tokio::io::AsyncBufReadExt;
use tokio::time::Instant;

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
    max_count: usize,
}

struct HttpJsonRpcProvider {
    next_id: AtomicUsize,
    client: reqwest::Client,
    url: Url,
}

impl HttpJsonRpcProvider {
    fn new(url: Url, client: reqwest::Client) -> Self {
        Self {
            next_id: 1.into(),
            client,
            url,
        }
    }

    /*
    /// this intentionally does no json parsing of the response
    /// TODO: make this generic. don't return Value
    pub async fn request(&self, method: &str, params: &Value) -> anyhow::Result<Value> {
        let next_id = self.next_id.fetch_add(1, atomic::Ordering::SeqCst);

        let request = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
            "id": next_id,
        });

        let response = self.send_supposed_json(request.to_string()).await?;

        let response_json = serde_json::from_str(&response)?;

        Ok(response_json)
    }
    */

    /// this sends any String but it's supposed to be json
    /// this allows us to test intentional errors
    /// TODO: make this generic. don't return String
    #[inline]
    async fn send_supposed_json(&self, request: String) -> Result<String, reqwest::Error> {
        self.client
            .post(self.url.clone())
            .header("content-type".to_string(), "application/json".to_string())
            .body(request)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await
    }
}

struct App {
    http_providers: Vec<HttpJsonRpcProvider>,
    max_count: usize,
}

impl App {
    async fn new(config: VersusConfig) -> anyhow::Result<Self> {
        let mut http_providers = vec![];

        // TODO: configure this with timeouts and such
        let c = reqwest::Client::new();

        for rpc in config.rpcs.iter() {
            match Url::parse(rpc) {
                Ok(url) => {
                    let provider = HttpJsonRpcProvider::new(url, c.clone());

                    http_providers.push(provider);
                }
                Err(err) => {
                    println!("Failed parsing url for {}: {:#?}", rpc, err);
                }
            }
        }

        let x = Self {
            http_providers,
            max_count: config.max_count,
        };

        Ok(x)
    }

    /// read jsonrpc lines from stdin and send to all the providers
    /// TODO: take a BufReader as input
    async fn run(&self) -> anyhow::Result<()> {
        // TODO: first check all of their chain ids

        let stdin = io::stdin();

        let reader = io::BufReader::new(stdin);

        let mut lines = reader.lines();

        let mut count = 0;

        while let Some(line) = lines.next_line().await? {
            self.send_supposed_json(line).await?;
            count += 1;

            if count >= self.max_count {
                break;
            }
        }

        println!("sent {}/{} requests", count, self.max_count);

        Ok(())
    }

    async fn send_supposed_json(&self, request: String) -> anyhow::Result<()> {
        // TODO: collect timings
        let requests = self.http_providers.iter().map(|provider| {
            let request = request.clone();
            let start = Instant::now();
            async move {
                let response = provider.send_supposed_json(request).await;

                let elapsed = start.elapsed();

                (provider, response, elapsed)
            }
        });

        let responses = join_all(requests).await;

        // TODO: i think we also need a HashMap of response -> Vec<Provider>
        let mut successes: Counter<String, usize> = Counter::new();
        let mut errors: Counter<String, usize> = Counter::new();

        for (provider, response, duration) in responses {
            match response {
                Ok(response) => {
                    successes[&response] += 1;
                }
                Err(err) => {
                    let err = format!("{:#?}", err);

                    errors[&err] += 1;
                }
            }

            println!("{} completed in {} ms", provider.url, duration.as_millis());
        }

        if errors.len() == 0 && successes.len() == 1 {
            println!("all matched! yey!");
            return Ok(());
        }

        println!("successes: {:#?}", successes);
        println!("errors: {:#?}", errors);

        Ok(())
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config: VersusConfig = argh::from_env();

    let app = App::new(config).await?;

    // TODO: optional timeout
    app.run().await?;

    /*
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

     */

    Ok(())
}
