use jsonrpsee::core::RpcResult;
use jsonrpsee::types::error::ErrorObject;
use std::sync::Arc;
pub use rpc_core_docker_registry::DockerRegistryApiServer;
use sp_runtime::traits::Block as BlockT;
use sp_blockchain::HeaderBackend;
use fp_rpc::EthereumRuntimeRPCApi;
use sp_api::ProvideRuntimeApi;

/// Net API implementation.
pub struct DockerRegistryImpl<B: BlockT, C> {
	_client: Arc<C>,
	_phantom_data: std::marker::PhantomData<B>,          
}

impl<B: BlockT, C> DockerRegistryImpl<B, C> {
	pub fn new(_client: Arc<C>) -> Self {
		Self {
			_client,
			_phantom_data: Default::default(),
		}
	}
}

impl<B, C> DockerRegistryApiServer for DockerRegistryImpl<B, C>
where
	B: BlockT,
	C: ProvideRuntimeApi<B> + 'static,
	C::Api: EthereumRuntimeRPCApi<B>,
	C: HeaderBackend<B> + Send + Sync,
{
    fn download_ipfs_file(&self, cid: String) -> RpcResult<Vec<u8>> {
        // Fetch the image from IPFS using the CID
        let image_data = fetch_image_from_ipfs(&cid)
            .map_err(|e| {
                ErrorObject::owned(
                    -32400, // Generic error code
                    "Failed to fetch IPFS file",
                    Some(format!("Error: {}", e))
                )
            })?;

        // Check if image data is empty
        if image_data.is_empty() {
            return Err(ErrorObject::owned(
                -32400, // Error code
                "Failed to fetch IPFS file", // Error message
                Some(format!("Error: No data returned from IPFS")) // Explicitly cast to `Option<String>`
            ));            
        }

        // Return the image data (raw bytes)
        Ok(image_data)
    }

}


fn fetch_image_from_ipfs(cid: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    // Use an IPFS client to fetch the image data
    let client = reqwest::blocking::Client::new();
    
    // Make sure the IPFS gateway URL is accessible and correct
    let url = format!("https://ipfs.io/ipfs/{}", cid);
    let response = client.get(&url)
        .send()?;
    
    // Check if the response is successful
    if !response.status().is_success() {
        return Err(Box::new(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("Failed to fetch image from IPFS, status code: {}", response.status())
        )));
    }
    
    let image_data = response.bytes()?.to_vec();
    
    // If no data is returned, handle this gracefully
    if image_data.is_empty() {
        return Err(Box::new(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "No data received from IPFS"
        )));
    }

    Ok(image_data)
}
