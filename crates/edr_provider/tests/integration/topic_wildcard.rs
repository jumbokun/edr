#![cfg(feature = "test-utils")]

//! An empty array in a log filter's `topics` is a wildcard for that position,
//! like `null`. See the execution-apis `eth_getLogs/topic-wildcard` test.

use std::str::FromStr as _;

use edr_chain_l1::{rpc::TransactionRequest, L1ChainSpec};
use edr_eth::{
    filter::{LogFilterOptions, OneOrMore},
    BlockSpec,
};
use edr_primitives::{Address, Bytes, B256, U256};
use edr_provider::{
    test_utils::{create_test_config, deploy_contract},
    time::CurrentTime,
    MethodInvocation, Provider, ProviderRequest,
};
use edr_signer::public_key_to_address;
use serde_json::Value;

use crate::common::provider::new_provider_from_config;

/// Deployment code for a contract whose fallback emits one log with topics
/// `0xaa` and `0xbb`: `MSTORE(0, 1); LOG2(0, 32, 0xaa, 0xbb)`.
const LOG2_EMITTER_DEPLOYMENT_BYTECODE: &str =
    "0x600f600c600039600f6000f3600160005260bb60aa60206000a200";

struct Fixture {
    provider: Provider<L1ChainSpec, CurrentTime>,
    caller: Address,
    contract: Address,
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
        let contract = deploy_contract(
            &provider,
            caller,
            Bytes::from_str(LOG2_EMITTER_DEPLOYMENT_BYTECODE)?,
        )?;

        Ok(Self {
            provider,
            caller,
            contract,
        })
    }

    fn request(&self, method: MethodInvocation<L1ChainSpec>) -> anyhow::Result<Value> {
        Ok(self
            .provider
            .handle_request(ProviderRequest::with_single(method))?
            .deserialize_result()?)
    }

    fn emit_log(&self) -> anyhow::Result<()> {
        self.request(MethodInvocation::SendTransaction(TransactionRequest {
            from: self.caller,
            to: Some(self.contract),
            ..TransactionRequest::default()
        }))?;

        Ok(())
    }
}

fn topic(byte: u8) -> B256 {
    B256::from(U256::from(byte))
}

fn filter(topics: Vec<Option<OneOrMore<B256>>>) -> LogFilterOptions {
    LogFilterOptions {
        from_block: Some(BlockSpec::Number(0)),
        topics: Some(topics),
        ..LogFilterOptions::default()
    }
}

fn empty() -> Option<OneOrMore<B256>> {
    Some(OneOrMore::Many(Vec::new()))
}

#[tokio::test(flavor = "multi_thread")]
async fn get_logs_treats_an_empty_topic_array_as_a_wildcard() -> anyhow::Result<()> {
    let fixture = Fixture::new()?;
    fixture.emit_log()?;

    let second_topic = Some(OneOrMore::Many(vec![topic(0xbb)]));
    for topics in [
        vec![empty(), second_topic.clone()],
        vec![None, second_topic],
        vec![empty()],
        vec![empty(), empty()],
    ] {
        let logs = fixture.request(MethodInvocation::GetLogs(filter(topics.clone())))?;
        assert_eq!(
            logs.as_array().map(Vec::len),
            Some(1),
            "topics {topics:?} should match the log"
        );
    }

    // The log has two topics, so a filter on a third position still excludes it.
    let logs = fixture.request(MethodInvocation::GetLogs(filter(vec![
        empty(),
        empty(),
        empty(),
    ])))?;
    assert_eq!(logs, Value::Array(Vec::new()));

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn log_filter_treats_an_empty_topic_array_as_a_wildcard() -> anyhow::Result<()> {
    let fixture = Fixture::new()?;

    let filter_id = fixture.request(MethodInvocation::NewFilter(filter(vec![
        empty(),
        Some(OneOrMore::One(topic(0xbb))),
    ])))?;
    let filter_id: U256 = serde_json::from_value(filter_id)?;

    fixture.emit_log()?;

    let changes = fixture.request(MethodInvocation::GetFilterChanges(filter_id))?;
    assert_eq!(changes.as_array().map(Vec::len), Some(1));

    Ok(())
}
