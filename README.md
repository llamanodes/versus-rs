# versus-rs
Versus takes a stream of requests and runs them against multiple endpoints simultaneously, comparing the output and timing.

Works similarly to https://github.com/inFURA/versus, but written in rust.

## Usage

  ```bash
  ethspam --rpc https://ethereum.llamarpc.com | head -n10 | cargo run -- https://ethereum.llamarpc.com https://ethereum-staging.llamarpc.com https://rpc.ankr.com/eth
  ```
