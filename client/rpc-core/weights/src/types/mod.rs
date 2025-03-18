
use serde::Deserialize;

#[derive(Deserialize)]
pub struct SubmitWeightsParams {
    pub netuid: u16,
    pub dests: Vec<u16>,
    pub weights: Vec<u16>,
    pub version_key: u32,
    pub default_spec_version: u32,
    pub default_genesis_hash: String, 
    pub finney_api_url: String, 
}