#![cfg(feature = "test-utils")]

//! Log queries whose `toBlock` is past the chain head.
//!
//! Blocks past the head don't exist yet, so they can't hold logs, but the range
//! comes straight from the request. A `toBlock` of `u64::MAX` used to make the
//! blockchain visit every block number up to it, which never finishes.

use std::{cell::Cell, mem, str::FromStr as _, sync::mpsc, time::Duration};

use edr_chain_l1::{rpc::TransactionRequest, L1ChainSpec};
use edr_eth::{filter::LogFilterOptions, BlockSpec};
use edr_primitives::{Address, Bytes, U256, U64};
use edr_provider::{
    test_utils::{create_test_config, deploy_contract},
    time::CurrentTime,
    MethodInvocation, Provider, ProviderRequest, ResponseWithCallTraces,
};
use edr_signer::public_key_to_address;

use crate::common::provider::new_provider_from_config;

/// Generous: a capped range responds immediately, so this only has to turn a
/// hang into a failure.
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(60);

/// Deployment code for a contract whose fallback emits one anonymous log:
/// `MSTORE(0, 1); LOG0(0, 32)`.
const LOG_EMITTER_DEPLOYMENT_BYTECODE: &str = "0x600b600c600039600b6000f3600160005260206000a000";

struct Fixture {
    /// Always `Some`, until [`Fixture`] is dropped.
    provider: Option<Provider<L1ChainSpec, CurrentTime>>,
    caller: Address,
    contract: Address,
    timed_out: Cell<bool>,
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
            provider: Some(provider),
            caller,
            contract,
            timed_out: Cell::new(false),
        })
    }

    fn provider(&self) -> &Provider<L1ChainSpec, CurrentTime> {
        self.provider
            .as_ref()
            .expect("the provider is only taken on drop")
    }

    fn emit_log(&self) -> anyhow::Result<()> {
        self.provider()
            .handle_request(ProviderRequest::with_single(
                MethodInvocation::SendTransaction(TransactionRequest {
                    from: self.caller,
                    to: Some(self.contract),
                    ..TransactionRequest::default()
                }),
            ))?;

        Ok(())
    }

    /// Sends `method` and waits at most [`RESPONSE_TIMEOUT`] for the response.
    fn request_with_timeout(
        &self,
        method: MethodInvocation<L1ChainSpec>,
    ) -> anyhow::Result<ResponseWithCallTraces> {
        let (sender, receiver) = mpsc::channel();

        self.provider().enqueue_request(
            ProviderRequest::with_single(method),
            Box::new(move |response| {
                let _ = sender.send(response);
            }),
        );

        let response = receiver.recv_timeout(RESPONSE_TIMEOUT);
        self.timed_out.set(response.is_err());

        Ok(response??)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // Dropping the provider joins its event loop, which is still busy with
        // the request that timed out. Leak it, so the test fails rather than
        // hangs.
        if self.timed_out.get() {
            #[allow(clippy::mem_forget)]
            mem::forget(self.provider.take());
        }
    }
}

fn filter_to_max_block() -> LogFilterOptions {
    LogFilterOptions {
        from_block: Some(BlockSpec::Number(0)),
        to_block: Some(BlockSpec::Number(u64::MAX)),
        ..LogFilterOptions::default()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn get_logs_with_to_block_past_head() -> anyhow::Result<()> {
    let fixture = Fixture::new()?;
    fixture.emit_log()?;

    let logs: Vec<serde_json::Value> = fixture
        .request_with_timeout(MethodInvocation::GetLogs(filter_to_max_block()))?
        .deserialize_result()?;

    assert_eq!(logs.len(), 1);

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn new_filter_with_to_block_past_head() -> anyhow::Result<()> {
    let fixture = Fixture::new()?;
    fixture.emit_log()?;

    let filter_id: U256 = fixture
        .request_with_timeout(MethodInvocation::NewFilter(filter_to_max_block()))?
        .deserialize_result()?;

    let logs: Vec<serde_json::Value> = fixture
        .request_with_timeout(MethodInvocation::GetFilterLogs(filter_id))?
        .deserialize_result()?;

    assert_eq!(logs.len(), 1);

    // Only the initial scan is capped. The filter still matches logs in blocks
    // mined after it was created.
    fixture.emit_log()?;

    let latest_block: U64 = fixture
        .request_with_timeout(MethodInvocation::BlockNumber(()))?
        .deserialize_result()?;

    let changes: Vec<serde_json::Value> = fixture
        .request_with_timeout(MethodInvocation::GetFilterChanges(filter_id))?
        .deserialize_result()?;

    let latest_block = format!("{latest_block:#x}");
    assert!(
        changes.iter().any(|log| log["blockNumber"] == latest_block),
        "no log from block {latest_block} in {changes:?}"
    );

    Ok(())
}
