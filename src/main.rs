use anyhow::Result;
use std::{
    env,
    io::{self, Write},
};

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
        Ok(_) => println!("Wallet loaded successfully."),
        Err(e) => println!("Error loading wallet: {:?}", e),
    }

    let funding_address = bitcoin_rpc.get_new_address(None, None).unwrap();
    let funding_address = funding_address.require_network(Network::Regtest).unwrap();

    let cold_storage_address = bitcoin_rpc.get_new_address(None, None).unwrap();
    let cold_storage_address = cold_storage_address
        .require_network(Network::Regtest)
        .unwrap();

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

    println!("🥶 basement fridge address: {:?}", cold_storage_address);

    println!(
        "🧊 kitchen fridge (CTV Vault Address: {:?})",
        ctv_vault_address
    );
    println!(
        "🍴 Kitchen (CTV Unvault Address: {:?})",
        ctv_unvault_address
    );

    println!(
        "\n😋 You start making a delicious reuben sandwich...! You look forward to eating it later, but hope your evil cat doesnt try to steal it first (Mining 101 blocks to funding address: {:?})",
        funding_address
    );
    let _ = bitcoin_rpc.generate_to_address(101, &funding_address);

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

    let funding_txid = match txid_result {
        Ok(txid) => {
            println!("\n🥪 You place the reuben sandwich in the kitchen fridge (Funding transaction sent: {})", txid);
            txid
        }
        Err(e) => {
            eprintln!("\n⚠️ Error sending funding transaction: {:?}", e);
            return;
        }
    };

    println!("\n Press Enter to continue...");
    let mut input = String::new();
    let _ = io::stdin().read_line(&mut input);

    println!("⌛ Some more time passes... (Mining 1 block)");
    let _ = bitcoin_rpc.generate_to_address(1, &funding_address);

    let spend_vault_tx = spend_ctv(
        funding_txid,
        amount - Amount::from_sat(420),
        vault_spend_info,
        ctv_unvault_address,
        None,
    );

    let serialized_tx = serialize_hex(&spend_vault_tx);

    let vault_spend_txid = bitcoin_rpc.send_raw_transaction(serialized_tx).unwrap();

    println!("\n🚨 Someone took the reuben sandwich out of the fridge!! or something...😹 (Transaction from vault sent to unvault address: TXID {})", vault_spend_txid);

    println!("\n Press Enter to continue...");
    let mut input = String::new();
    let _ = io::stdin().read_line(&mut input);

    // Check here if txid is in mempool

    println!("🕗 Even more time passes... (Mining 1 block)");
    let _ = bitcoin_rpc.generate_to_address(1, &funding_address);

    let tx_outs = bitcoin_rpc.get_tx_out(&vault_spend_txid, 0, None);

    match tx_outs {
        Ok(tx_outs) => {
            println!("\n📲 Your super smart fridge texts you the following alert: Your delicious reuben sandwich has been taken from the fridge and moved in to the kitchen! (TXID {vault_spend_txid} FOUND IN MEMPOOL!!)");

            println!("\n Press Enter to continue...");
            let mut input = String::new();
            let _ = io::stdin().read_line(&mut input);

            println!("\nWhat do you want to do? (enter a number between 1 and 3) \n\n1: Run to the kitchen to rescue the reuben sandwich and take it to the really cold fridge in your basement (sweep funds to cold storage address)\n2: Put the sandwich in the toaster and wait for it to heat up (sweep funds to hot wallet address)\n3:😹 You are the evil cat thief so you want to take the sandwich for yourself (sweep funds to theifs address)");
            io::stdout().flush().unwrap();
            let mut input = String::new();
            io::stdin()
                .read_line(&mut input)
                .expect("Failed to read input");

            let answer = input.trim();

            // Spend from unvault contract to hot wallet

            let hot_wallet_addr: Address<bitcoin::address::NetworkUnchecked> =
                bitcoin_rpc.get_new_address(None, None).unwrap();
            let hot_wallet_addr = hot_wallet_addr.require_network(Network::Regtest).unwrap();

            let prev_outs = vec![TxOut {
                value: tx_outs.clone().unwrap().value,
                script_pubkey: tx_outs.clone().unwrap().script_pub_key.script().unwrap(),
            }];

            let unvault_tx = spend_to_hot(
                vault_spend_txid,
                tx_outs.unwrap().value,
                hot_wallet_addr.clone(),
                hot_wallet_pubkey,
                unvault_spend_info.clone(),
                &prev_outs,
                hot_wallet_key_pair.secret_key(),
            );

            let serialized_tx = serialize_hex(&unvault_tx);

            match answer {
                "1" => println!("\n🏃 You make a run for the kitchen to investigate!!"),
                "2" => {
                    println!("\n🍞 You wait for the reuben sandwich to heat up in the toaster while keeping an eye out for wizards or cats or something...idk (mining 101 blocks) ");
                    let _ = bitcoin_rpc.generate_to_address(101, &funding_address);
                }
                "3" => println!(
                    "\n😹 You are the evil cat but you have standards and you still want to heat up your sandwich in the toaster so now you have to wait"
                ),
                _ => println!("\n Invalid option selected!"),
            }

            let hot_wallet_txid = bitcoin_rpc.send_raw_transaction(serialized_tx);

            if hot_wallet_txid.is_err() {
                println!("\n Press Enter to continue...");

                let mut input = String::new();
                let _ = io::stdin().read_line(&mut input);

                println!("\n⏳ The evil cat tried to eat the sandwich but it's not ready to eat yet!!! (hot wallet spend path failed as 100 blocks have not passed): {:?}", hot_wallet_txid);

                println!("\n Press Enter to continue...");

                let mut input = String::new();
                let _ = io::stdin().read_line(&mut input);

                println!("\nChoose an option below:\n1: Put the sandwich in the really cold fridge in your basement so you can eat it later (sweep funds to cold storage address)\n2:😹 You are the evil cat thief, if you cant eait it now you then want to take the sandwich home to your own fridge and eat it later (try and sweep funds to a different cold storage address)");

                io::stdout().flush().unwrap();
                let mut input = String::new();
                io::stdin()
                    .read_line(&mut input)
                    .expect("Failed to read input");

                let answer_2 = input.trim();
                if answer_2 == "1" {
                    let spend_unvault_tx_to_cold = spend_ctv(
                        vault_spend_txid,
                        amount - Amount::from_sat(840),
                        unvault_spend_info,
                        cold_storage_address,
                        None,
                    );

                    let serialized_tx = serialize_hex(&spend_unvault_tx_to_cold);

                    let txid = bitcoin_rpc.send_raw_transaction(serialized_tx).unwrap();

                    println!("\n❄️ You put the sandwich in the really cold fridge in your basement so you can eat it later. You think to yourself that its really amazing how i can secure this sandwich so easily and not have to worry about backing up any crazy weird stuff to recreat the sandwich. You also think to yourself that it was a weird thing to think about (Transaction from vault to cold storage sent: {})", txid);
                } else {
                    let spend_unvault_tx_to_cold = spend_ctv(
                        vault_spend_txid,
                        amount - Amount::from_sat(840),
                        unvault_spend_info,
                        cold_storage_address,
                        Some(hot_wallet_addr),
                    );

                    let serialized_tx = serialize_hex(&spend_unvault_tx_to_cold);

                    let failed_txid = bitcoin_rpc.send_raw_transaction(serialized_tx);

                    if failed_txid.is_err() {
                        println!("\n🚫 LOOOOL NICE TRY STUPID CAT THIEF, FOR SOME REASON THIS SANDWICH CAN ONLY GO IN THE OWNER'S FRIDGE!! OR YOU HAVE TO WAIT FOR THE TOASTER TO HEAT IT UP TO EAT IT !! (can't send to any address other than the one specified in the CTV contract, or wait 100 blocks) {:?}", failed_txid);
                    }
                }
            } else {
                println!("\n Press Enter to continue...");
                let mut input = String::new();
                let _ = io::stdin().read_line(&mut input);
                println!("\n🏃 you somehow got lost on the way to your own kitchen, or you are not infact a theif and you are standing next to the toaster and it just pinged. The sandwich is now ready to eat. (the CSV timelock of 100 blocks passed)");
                println!("\n Press Enter to continue...");
                let mut input = String::new();
                let _ = io::stdin().read_line(&mut input);
                if answer == "2" {
                    println!("\n🍞 You take the sandwich out of the toaster and eat it. It tasted really good and you think to yourself that we should defo enable these sandwiches asap, (funds have been swept to hot wallet: {})", hot_wallet_txid.unwrap());
                } else if answer == "3" {
                    println!("\n😹 The evil cat thief eat the sandwich!! 😹🫵 , (funds have been swept to theifs hot wallet f: {})", hot_wallet_txid.unwrap());
                }
            }
        }
        Err(e) => {
            eprintln!("\n⚠️ Error getting tx outs: {:?}", e);
        }
    }
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
    test_wrong_addr: Option<Address>,
) -> Transaction {
    let inputs = vec![TxIn {
        previous_output: OutPoint { txid, vout: 0 },
        sequence: Sequence::default(),
        ..Default::default()
    }];

    let ctv_tx_out = vec![TxOut {
        value: amount,
        script_pubkey: ctv_target_address.script_pubkey(),
    }];

    let mut unsigned_tx = if test_wrong_addr.is_none() {
        Transaction {
            version: transaction::Version::TWO,
            lock_time: absolute::LockTime::ZERO,
            input: inputs,
            output: ctv_tx_out.clone(),
        }
    } else {
        let ctv_tx_out = vec![TxOut {
            value: amount,
            script_pubkey: test_wrong_addr.unwrap().script_pubkey(),
        }];
        Transaction {
            version: transaction::Version::TWO,
            lock_time: absolute::LockTime::ZERO,
            input: inputs,
            output: ctv_tx_out.clone(),
        }
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
        value: amount - Amount::from_sat(840),
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
