# Op_SecureTheSandwich: A CTV Vault RPG

## Overview

Can you stop the evil cat thief from eating the Reuben sandwich before it's too late?! 🥪🐱

## Requirements

- Rust
- Bitcoin Inquisition node https://github.com/bitcoin-inquisition/bitcoin

## Commands

To start the game, run the following commands

Start a bitcoin inquisition node in regtest (i hadd to add fallback fee for now to get it to work)
```bash
./bitcoind -regtest -fallbackfee=0.0001
```

```bash
cargo run
```

## Example



## Tech stuff

This is a reimplentation of jamesob's `simple-ctv-vault`. You can read more here https://github.com/jamesob/simple-ctv-vault


```mermaid
flowchart TD
  A(UTXO you want to vault) -->|"[some spend] e.g. P2WPKH"| V(to_vault_tx<br/>Coins are now vaulted)
  V -->|"<code>&lt;H(unvault_tx)&gt; OP_CHECKTEMPLATEVERIFY</code>"| U(unvault_tx<br/>Begin the unvaulting process)
  U -->|"(cold sweep)<br/><code>&lt;H(tocold_tx)&gt; OP_CHECKTEMPLATEVERIFY</code>"| C(tocold_tx)
  U -->|"(delayed hot spend)<br/><code>&lt;block_delay&gt; OP_CSV<br />&lt;hot_pubkey&gt; OP_CHECKSIG</code>"| D(<code>tohot_tx</code>)
  C -->|"<code>&lt;cold_pubkey&gt; OP_CHECKSIG</code>"| E(some undefined destination)
```


The ctv hash script i used is from here https://github.com/bennyhodl/dlcat


### TODO:

fees are hard coded for now so i need figure out how to add ephemeral anchors https://bitcoinops.org/en/topics/ephemeral-anchors/
