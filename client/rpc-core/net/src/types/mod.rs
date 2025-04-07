// use ethereum_types::{H512, U256};
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
#[serde(untagged)]
pub enum PeerCount {
	U32(u32),
	String(String),
}
