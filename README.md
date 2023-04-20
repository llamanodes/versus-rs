# versus-rs
Versus takes a stream of requests and runs them against multiple endpoints simultaneously, comparing the output and timing.

Works similarly to https://github.com/inFURA/versus, but written in rust.

## Usage

  ```bash
  cargo run -- http://127.0.0.1:8545 https://eth.llamarpc.com
  ```
