
use serde::Deserialize;

#[derive(Deserialize)]
pub struct SubmitWeightsParams {
    pub netuid: u16,
    pub dests: Vec<u16>,
    pub weights: Vec<u16>,
    pub version_key: u64,
}