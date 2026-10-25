#![cfg(feature = "test-utils")]

//! `eth_getFilterLogs` returns every log matching the filter, every time. It
//! used to hand over the logs pending for `eth_getFilterChanges` instead, so a
//! second call, or a poll after it, came back empty.

use std::str::FromStr as _;

use edr_chain_l1::{rpc::TransactionRequest, L1ChainSpec};
use edr_eth::{filter::LogFilterOptions, BlockSpec};
use edr_primitives::{Address, Bytes, U256};
use edr_provider::{
    test_utils::{create_test_config, deploy_contract},
    time::CurrentTime,
    MethodInvocation, Provider, ProviderRequest,
};
use edr_signer::public_key_to_address;

use crate::common::provider::new_provider_from_config;

/// Deployment code for a contract whose fallback emits one anonymous log:
/// `MSTORE(0, 1); LOG0(0, 32)`.
const LOG_EMITTER_DEPLOYMENT_BYTECODE: &str = "0x600b600c600039600b6000f3600160005260206000a000";

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
            Bytes::from_str(LOG_EMITTER_DEPLOYMENT_BYTECODE)?,
        )?;

        Ok(Self {
            provider,
            caller,
            contract,
        })
    }

    fn emit_log(&self) -> anyhow::Result<()> {
        self.provider.handle_request(ProviderRequest::with_single(
            MethodInvocation::SendTransaction(TransactionRequest {
                from: self.caller,
                to: Some(self.contract),
                ..TransactionRequest::default()
            }),
        ))?;

        Ok(())
    }

    fn logs(
        &self,
        method: MethodInvocation<L1ChainSpec>,
    ) -> anyhow::Result<Vec<serde_json::Value>> {
        Ok(self
            .provider
            .handle_request(ProviderRequest::with_single(method))?
            .deserialize_result()?)
    }
}

fn from_genesis() -> LogFilterOptions {
    LogFilterOptions {
        from_block: Some(BlockSpec::Number(0)),
        ..LogFilterOptions::default()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn get_filter_logs_returns_all_matching_logs_every_time() -> anyhow::Result<()> {
    let fixture = Fixture::new()?;
    fixture.emit_log()?;

    let filter_id: U256 = fixture
        .provider
        .handle_request(ProviderRequest::with_single(MethodInvocation::NewFilter(
            from_genesis(),
        )))?
        .deserialize_result()?;

    for _ in 0..2 {
        let logs = fixture.logs(MethodInvocation::GetFilterLogs(filter_id))?;
        assert_eq!(logs.len(), 1);
    }

    // The log is still pending for the filter's first poll.
    let changes = fixture.logs(MethodInvocation::GetFilterChanges(filter_id))?;
    assert_eq!(changes.len(), 1);

    fixture.emit_log()?;

    let filter_logs = fixture.logs(MethodInvocation::GetFilterLogs(filter_id))?;
    let logs = fixture.logs(MethodInvocation::GetLogs(from_genesis()))?;
    assert_eq!(filter_logs.len(), 2);
    assert_eq!(filter_logs, logs);

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn get_filter_logs_rejects_a_block_filter() -> anyhow::Result<()> {
    let fixture = Fixture::new()?;

    let filter_id: U256 = fixture
        .provider
        .handle_request(ProviderRequest::with_single(
            MethodInvocation::NewBlockFilter(()),
        ))?
        .deserialize_result()?;

    let result = fixture
        .provider
        .handle_request(ProviderRequest::with_single(
            MethodInvocation::GetFilterLogs(filter_id),
        ));

    assert!(result.is_err());

    Ok(())
}
