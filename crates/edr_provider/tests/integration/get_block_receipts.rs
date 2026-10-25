#![cfg(feature = "test-utils")]

//! `eth_getBlockReceipts`.

use edr_chain_l1::{rpc::TransactionRequest, L1ChainSpec};
use edr_eth::{BlockSpec, Eip1898BlockSpec, PreEip1898BlockSpec};
use edr_primitives::{Address, B256, U256};
use edr_provider::{
    test_utils::create_test_config, time::CurrentTime, MethodInvocation, Provider, ProviderRequest,
};
use edr_signer::public_key_to_address;
use serde_json::Value;

use crate::common::provider::new_provider_from_config;

struct Fixture {
    provider: Provider<L1ChainSpec, CurrentTime>,
    caller: Address,
}

impl Fixture {
    fn new() -> anyhow::Result<Self> {
        let config = create_test_config();

        let caller = public_key_to_address(
            config
                .owned_accounts
                .first()
                .expect("config should have an account")
                .public_key(),
        );

        let provider = new_provider_from_config(config)?;

        Ok(Self { provider, caller })
    }

    fn request(&self, method: MethodInvocation<L1ChainSpec>) -> anyhow::Result<Value> {
        Ok(self
            .provider
            .handle_request(ProviderRequest::with_single(method))?
            .deserialize_result()?)
    }

    fn send_transfer(&self) -> anyhow::Result<B256> {
        let hash = self.request(MethodInvocation::SendTransaction(TransactionRequest {
            from: self.caller,
            to: Some(Address::repeat_byte(0x11)),
            value: Some(U256::from(1)),
            // The test config's default gas limit fills a whole block.
            gas: Some(21_000),
            ..TransactionRequest::default()
        }))?;

        Ok(serde_json::from_value(hash)?)
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn returns_the_receipt_of_each_transaction_in_the_block() -> anyhow::Result<()> {
    let fixture = Fixture::new()?;

    fixture.request(MethodInvocation::EvmSetAutomine(false))?;
    let transaction_hashes = [fixture.send_transfer()?, fixture.send_transfer()?];
    fixture.request(MethodInvocation::EvmMine(None))?;

    let receipts = fixture.request(MethodInvocation::GetBlockReceipts(BlockSpec::latest()))?;

    let expected = transaction_hashes
        .iter()
        .map(|hash| fixture.request(MethodInvocation::GetTransactionReceipt(*hash)))
        .collect::<anyhow::Result<Vec<_>>>()?;

    assert_eq!(receipts, Value::Array(expected));

    let block = fixture.request(MethodInvocation::GetBlockByNumber(
        PreEip1898BlockSpec::latest(),
        false,
    ))?;
    let block_hash: B256 = serde_json::from_value(block["hash"].clone())?;

    let receipts_by_hash = fixture.request(MethodInvocation::GetBlockReceipts(
        BlockSpec::Eip1898(Eip1898BlockSpec::Hash {
            block_hash,
            require_canonical: None,
        }),
    ))?;

    assert_eq!(receipts_by_hash, receipts);

    // The execution-apis tests pass the hash as a bare string.
    let json = format!(r#"{{"method":"eth_getBlockReceipts","params":["{block_hash}"]}}"#);
    let request: MethodInvocation<L1ChainSpec> = serde_json::from_str(&json)?;
    assert_eq!(fixture.request(request)?, receipts);

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn returns_an_empty_array_for_a_block_without_transactions() -> anyhow::Result<()> {
    let fixture = Fixture::new()?;

    let receipts = fixture.request(MethodInvocation::GetBlockReceipts(BlockSpec::earliest()))?;

    assert_eq!(receipts, Value::Array(Vec::new()));

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn returns_null_for_an_unknown_block() -> anyhow::Result<()> {
    let fixture = Fixture::new()?;

    let by_number = fixture.request(MethodInvocation::GetBlockReceipts(BlockSpec::Number(100)))?;
    assert_eq!(by_number, Value::Null);

    let by_hash = fixture.request(MethodInvocation::GetBlockReceipts(BlockSpec::Eip1898(
        Eip1898BlockSpec::Hash {
            block_hash: B256::repeat_byte(0x22),
            require_canonical: None,
        },
    )))?;
    assert_eq!(by_hash, Value::Null);

    let json = format!(
        r#"{{"method":"eth_getBlockReceipts","params":["{}"]}}"#,
        B256::ZERO
    );
    let request: MethodInvocation<L1ChainSpec> = serde_json::from_str(&json)?;
    assert_eq!(fixture.request(request)?, Value::Null);

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn pending_returns_the_receipts_of_the_next_block() -> anyhow::Result<()> {
    let fixture = Fixture::new()?;

    fixture.request(MethodInvocation::EvmSetAutomine(false))?;
    let transaction_hash = fixture.send_transfer()?;

    let receipts = fixture.request(MethodInvocation::GetBlockReceipts(BlockSpec::pending()))?;

    let receipts = receipts.as_array().expect("receipts should be an array");
    assert_eq!(receipts.len(), 1);
    assert_eq!(
        receipts[0]["transactionHash"],
        serde_json::to_value(transaction_hash)?
    );

    Ok(())
}
