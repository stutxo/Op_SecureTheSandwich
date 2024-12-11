use anyhow::Result;
use std::env;

use bitcoin::{
    absolute,
    consensus::{encode::serialize_hex, Encodable},
    hashes::{sha256, Hash},
    key::{Keypair, Secp256k1},
    opcodes::all::{OP_CHECKSIG, OP_CSV, OP_DROP, OP_NOP4},
    script::Builder,
    secp256k1::{Message, SecretKey},
    sighash::{Prevouts, SighashCache},
    taproot::{LeafVersion, TaprootBuilder, TaprootSpendInfo},
    transaction, Address, Amount, Network, OutPoint, ScriptBuf, Sequence, TapLeafHash,
    TapSighashType, Transaction, TxIn, TxOut, Txid, XOnlyPublicKey,
};
use bitcoincore_rpc::{Auth, Client, RpcApi};

const PORT: &str = "18443";
const TIMEOUT: u32 = 100;

fn main() {
    let bitcoin_rpc_user = env::var("BITCOIN_RPC_USER").expect("BITCOIN_RPC_USER not set");
    let bitcoin_rpc_pass = env::var("BITCOIN_RPC_PASS").expect("BITCOIN_RPC_PASS not set");

    let wallet_name = "ctv_vault";

    let bitcoin_rpc_url = format!("http://localhost:{}/wallet/{}", PORT, wallet_name);

    let bitcoin_rpc = Client::new(
        &bitcoin_rpc_url,
        Auth::UserPass(bitcoin_rpc_user, bitcoin_rpc_pass),
    )
    .unwrap();

    let create_wallet = bitcoin_rpc.create_wallet(wallet_name, None, None, None, None);

    if create_wallet.is_ok() {
        println!("Wallet created successfully.");
    }

    let load_wallet = bitcoin_rpc.load_wallet(wallet_name);

    match load_wallet {
        Ok(_) => {
            println!("Wallet loaded successfully.");
        }
        Err(e) => {
            println!("Error loading wallet: {:?}", e);
        }
    }

    let funding_address = bitcoin_rpc.get_new_address(None, None).unwrap();
    let funding_address = funding_address.require_network(Network::Regtest).unwrap();

    let cold_storage_address = bitcoin_rpc.get_new_address(None, None).unwrap();
    let cold_storage_address = cold_storage_address
        .require_network(Network::Regtest)
        .unwrap();

    println!("Cold storage address: {:?}", cold_storage_address);

    let secp = Secp256k1::new();

    let hot_wallet_key_pair = Keypair::new(&secp, &mut rand::thread_rng());
    let (hot_wallet_pubkey, _parity) = XOnlyPublicKey::from_keypair(&hot_wallet_key_pair);

    let amount = Amount::from_btc(1.).unwrap();

    let unvault_spend_info =
        create_unvault_address(hot_wallet_pubkey, amount, cold_storage_address.clone()).unwrap();

    let ctv_unvault_address =
        Address::p2tr_tweaked(unvault_spend_info.output_key(), Network::Regtest);

    let vault_spend_info = create_vault_address(amount, ctv_unvault_address.clone()).unwrap();

    let ctv_vault_address = Address::p2tr_tweaked(vault_spend_info.output_key(), Network::Regtest);

    println!("CTV Vault Address: {:?}", ctv_vault_address);
    println!("CTV Unvault Address: {:?}", ctv_unvault_address);

    println!("Mining blocks to Address: {:?}...", funding_address);

    let _ = bitcoin_rpc.generate_to_address(110, &funding_address);

    let txid_result = bitcoin_rpc.send_to_address(
        &ctv_vault_address,
        amount,
        None,
        None,
        None,
        None,
        None,
        None,
    );

    let txid = match txid_result {
        Ok(txid) => {
            println!("Funding transaction sent: {}", txid);
            txid
        }
        Err(e) => {
            eprintln!("Error sending funding transaction: {:?}", e);
            return;
        }
    };

    let tx_outs = bitcoin_rpc.get_tx_out(&txid, 0, None).unwrap();

    let tx_outs = tx_outs.unwrap();
    let prev_outs = vec![TxOut {
        value: tx_outs.value,
        script_pubkey: tx_outs.script_pub_key.script().unwrap(),
    }];

    println!("Mining 10 blocks...");
    let _ = bitcoin_rpc.generate(10, None);

    let hot_wallet_addr: Address<bitcoin::address::NetworkUnchecked> =
        bitcoin_rpc.get_new_address(None, None).unwrap();
    let hot_wallet_addr = hot_wallet_addr.require_network(Network::Regtest).unwrap();

    let spend_vault_tx = spend_ctv(txid, amount, vault_spend_info, ctv_unvault_address);

    let serialized_tx = serialize_hex(&spend_vault_tx);

    let txid = bitcoin_rpc.send_raw_transaction(serialized_tx).unwrap();

    println!("Transaction from vault sent: {}", txid);

    // println!("Mining 10 blocks...");
    // let _ = bitcoin_rpc.generate(10, None);

    // let unvault_tx = spend_to_hot(
    //     txid,
    //     amount,
    //     hot_wallet_addr,
    //     hot_wallet_pubkey,
    //     unvault_spend_info,
    //     &prev_outs,
    //     hot_wallet_key_pair.secret_key(),
    // );
}

fn spend_to_hot(
    txid: Txid,
    amount: Amount,
    hot_address: Address,
    hot_wallet_pubkey: XOnlyPublicKey,
    taproot_spend_info: TaprootSpendInfo,
    prev_outs: &[TxOut],
    priv_key: SecretKey,
) -> Transaction {
    let secp = Secp256k1::new();

    let inputs = vec![TxIn {
        previous_output: OutPoint { txid, vout: 0 },
        sequence: Sequence(TIMEOUT),
        ..Default::default()
    }];

    let hot_tx_out = vec![TxOut {
        value: amount - Amount::from_sat(420),
        script_pubkey: hot_address.script_pubkey(),
    }];

    let mut unsigned_tx = Transaction {
        version: transaction::Version::TWO,
        lock_time: absolute::LockTime::ZERO,
        input: inputs,
        output: hot_tx_out,
    };

    let hot_spend_script = send_to_hot(hot_wallet_pubkey);

    let unsigned_tx_clone = unsigned_tx.clone();

    let tap_leaf_hash = TapLeafHash::from_script(&hot_spend_script, LeafVersion::TapScript);
    for (index, input) in unsigned_tx.input.iter_mut().enumerate() {
        let sighash = SighashCache::new(&unsigned_tx_clone)
            .taproot_script_spend_signature_hash(
                index,
                &Prevouts::All(prev_outs),
                tap_leaf_hash,
                TapSighashType::Default,
            )
            .expect("failed to construct sighash");

        let message = Message::from(sighash);
        let keypair = Keypair::from_secret_key(&secp, &priv_key);
        let signature = secp.sign_schnorr_no_aux_rand(&message, &keypair);

        let script_ver = (hot_spend_script.clone(), LeafVersion::TapScript);
        let ctrl_block = taproot_spend_info.control_block(&script_ver).unwrap();

        input.witness.push(signature.serialize());
        input.witness.push(script_ver.0.into_bytes());
        input.witness.push(ctrl_block.serialize());
    }
    unsigned_tx
}

fn spend_ctv(
    txid: Txid,
    amount: Amount,
    taproot_spend_info: TaprootSpendInfo,
    ctv_target_address: Address,
) -> Transaction {
    let inputs = vec![TxIn {
        previous_output: OutPoint { txid, vout: 0 },
        sequence: Sequence::default(),
        ..Default::default()
    }];

    let ctv_tx_out = vec![TxOut {
        value: amount - Amount::from_sat(420),
        script_pubkey: ctv_target_address.script_pubkey(),
    }];

    let mut unsigned_tx = Transaction {
        version: transaction::Version::TWO,
        lock_time: absolute::LockTime::ZERO,
        input: inputs,
        output: ctv_tx_out.clone(),
    };

    let ctv_hash = calc_ctv_hash(&ctv_tx_out, false);

    let ctv_script = send_ctv(ctv_hash);

    for input in unsigned_tx.input.iter_mut() {
        let script_ver = (ctv_script.clone(), LeafVersion::TapScript);
        let ctrl_block = taproot_spend_info.control_block(&script_ver).unwrap();

        input.witness.push(ctv_hash);
        input.witness.push(script_ver.0.into_bytes());
        input.witness.push(ctrl_block.serialize());
    }
    unsigned_tx
}

fn create_unvault_address(
    hot_wallet_pubkey: XOnlyPublicKey,
    amount: Amount,
    cold_storage_address: Address,
) -> Result<TaprootSpendInfo> {
    let secp = Secp256k1::new();

    let key_pair = Keypair::new(&secp, &mut rand::thread_rng());
    // Random unspendable XOnlyPublicKey provided for internal key
    let (unspendable_pubkey, _parity) = XOnlyPublicKey::from_keypair(&key_pair);

    let cold_tx_out = TxOut {
        value: amount - Amount::from_sat(420),
        script_pubkey: cold_storage_address.script_pubkey(),
    };

    let cold_spend_script = send_ctv(calc_ctv_hash(&[cold_tx_out], false));

    let hot_spend_script = send_to_hot(hot_wallet_pubkey);

    let taproot_spend_info = TaprootBuilder::new()
        .add_leaf(1, cold_spend_script)
        .unwrap()
        .add_leaf(1, hot_spend_script)
        .unwrap()
        .finalize(&secp, unspendable_pubkey)
        .unwrap();

    Ok(taproot_spend_info)
}

fn create_vault_address(amount: Amount, unvault_addr: Address) -> Result<TaprootSpendInfo> {
    let secp = Secp256k1::new();

    let key_pair = Keypair::new(&secp, &mut rand::thread_rng());
    // Random unspendable XOnlyPublicKey provided for internal key
    let (unspendable_pubkey, _parity) = XOnlyPublicKey::from_keypair(&key_pair);

    let ctv_tx_out = TxOut {
        value: amount - Amount::from_sat(420),
        script_pubkey: unvault_addr.script_pubkey(),
    };

    let ctv_script = send_ctv(calc_ctv_hash(&[ctv_tx_out], false));

    let taproot_spend_info = TaprootBuilder::new()
        .add_leaf(0, ctv_script)
        .unwrap()
        .finalize(&secp, unspendable_pubkey)
        .unwrap();

    Ok(taproot_spend_info)
}

fn calc_ctv_hash(outputs: &[TxOut], is_timout_script: bool) -> [u8; 32] {
    let mut buffer = Vec::new();
    buffer.extend(2_i32.to_le_bytes()); // version
    buffer.extend(0_i32.to_le_bytes()); // locktime
    buffer.extend(1_u32.to_le_bytes()); // inputs len

    let seq = if is_timout_script {
        sha256::Hash::hash(&Sequence(TIMEOUT).0.to_le_bytes())
    } else {
        sha256::Hash::hash(&Sequence::default().0.to_le_bytes())
    };
    buffer.extend(seq.to_byte_array()); // sequences

    let outputs_len = outputs.len() as u32;
    buffer.extend(outputs_len.to_le_bytes()); // outputs len

    let mut output_bytes: Vec<u8> = Vec::new();
    for o in outputs {
        o.consensus_encode(&mut output_bytes).unwrap();
    }
    buffer.extend(sha256::Hash::hash(&output_bytes).to_byte_array()); // outputs hash

    buffer.extend(0_u32.to_le_bytes()); // inputs index

    let hash = sha256::Hash::hash(&buffer);
    hash.to_byte_array()
}

fn send_ctv(ctv_hash: [u8; 32]) -> ScriptBuf {
    Builder::new()
        .push_slice(ctv_hash)
        .push_opcode(OP_NOP4)
        .push_opcode(OP_DROP)
        .into_script()
}

fn send_to_hot(pubkey: XOnlyPublicKey) -> ScriptBuf {
    Builder::new()
        .push_int(TIMEOUT as i64)
        .push_opcode(OP_CSV)
        .push_opcode(OP_DROP)
        .push_x_only_key(&pubkey)
        .push_opcode(OP_CHECKSIG)
        .into_script()
}
