import asyncio
import json
import logging
import ssl
import os
import yaml
import certifi
import bittensor as bt
from substrateinterface import SubstrateInterface, Keypair
from substrateinterface.utils.ss58 import ss58_decode
from substrateinterface.exceptions import SubstrateRequestException

# Fix SSL certificate issues on macOS
os.environ['SSL_CERT_FILE'] = certifi.where()
os.environ['SSL_CERT_DIR'] = certifi.where()

# Create a custom SSL context for better compatibility
def create_ssl_context():
    """Create a custom SSL context with proper certificate handling"""
    try:
        context = ssl.create_default_context(cafile=certifi.where())
        context.check_hostname = False
        context.verify_mode = ssl.CERT_NONE
        return context
    except Exception as e:
        # Can't use logger here yet, so just return None
        return None

# Setup logging
logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s - %(name)s - %(levelname)s - %(message)s"
)
logger = logging.getLogger(__name__)

# Additional SSL fixes for stubborn SSL issues
try:
    ssl._create_default_https_context = ssl._create_unverified_context
    # Also disable SSL verification for urllib3 and requests
    import urllib3
    urllib3.disable_warnings(urllib3.exceptions.InsecureRequestWarning)
    logger.info("SSL verification disabled as fallback")
except Exception as e:
    pass  # Ignore if this fails


U16_MAX = 65535


def dividend_to_u16(dividend: float) -> int:
    """Convert an SDK dividend (normalized 0..1 float) to the raw u16 the chain stores.

    Metagraph.submit_hot_keys_info takes `dividends: Vec<u16>`; passing the float
    directly would be truncated to 0 by the SCALE encoder.
    """
    return min(U16_MAX, max(0, round(float(dividend) * U16_MAX)))


class StorageMonitor:
    def __init__(self, config_path="config.yaml"):
        # Load configuration
        self.config = self.load_config(config_path)
        
        # Your chain connection - using WebSocket from config
        self.my_chain_url = self.config['hippius']['rpc_endpoint']
        # Bittensor chain connection - network name or websocket URL from config
        self.bittensor_network = self.config['bittensor']['chain_endpoint']
        
        self.my_chain = None
        self.bittensor_chain = None
        self.last_processed_block = 0
        self.net_uid = self.config['bittensor']['subnet_id']
        self.block_interval = 85  # submit every 85 blocks
        
        # Initialize keypair from environment variable
        mnemonic = os.getenv('BITTENSOR_MNEMONIC')
        if not mnemonic:
            raise ValueError("BITTENSOR_MNEMONIC environment variable is required")
        
        self.keypair = Keypair.create_from_mnemonic(mnemonic)
        logger.info(f"Using account: {self.keypair.ss58_address}")

    def load_config(self, config_path):
        """Load configuration from YAML file"""
        try:
            with open(config_path, 'r') as file:
                config = yaml.safe_load(file)
            logger.info(f"Configuration loaded from {config_path}")
            return config
        except FileNotFoundError:
            logger.error(f"Configuration file {config_path} not found")
            raise
        except yaml.YAMLError as e:
            logger.error(f"Error parsing configuration file: {e}")
            raise

    def connect_chains(self):
        """Try to connect to both chains with retry"""
        while True:
            try:
                logger.info(f"Connecting to your chain: {self.my_chain_url}")
                
                # Create SSL context for better compatibility
                ssl_context = create_ssl_context()
                
                try:
                    self.my_chain = SubstrateInterface(
                        url=self.my_chain_url,
                        use_remote_preset=True,
                        ss58_format=42,  # Add SS58 format for your chain
                        ssl_context=ssl_context
                    )
                except TypeError:
                    # Fallback if ssl_context parameter not supported
                    self.my_chain = SubstrateInterface(
                        url=self.my_chain_url,
                        use_remote_preset=True,
                        ss58_format=42  # Add SS58 format for your chain
                    )
                logger.info("✅ Connected to your chain")

                logger.info(f"Connecting to Bittensor chain ({self.bittensor_network}) using Bittensor library...")
                # bittensor 11.x: blocking when used directly, connects lazily on first call
                self.bittensor_chain = bt.Subtensor(network=self.bittensor_network)
                logger.info("✅ Connected to Bittensor chain using Bittensor library")

                return True

            except Exception as e:
                logger.error(f"Connection error: {e}")
                if "SSL" in str(e) or "ssl" in str(e):
                    logger.error("SSL-related error detected. This might be a certificate issue.")
                    logger.info("Trying to refresh SSL context...")
                    # Force SSL context refresh
                    try:
                        import ssl
                        ssl._create_default_https_context = ssl._create_unverified_context
                        logger.info("SSL context set to unverified mode")
                    except Exception as ssl_e:
                        logger.warning(f"Could not set unverified SSL context: {ssl_e}")
                
                logger.info("⏳ Retrying connection in 10s...")
                # Use time.sleep instead of asyncio.sleep for synchronous context
                import time
                time.sleep(10)

    def get_block_number(self, substrate):
        """Get the latest block number, or None if it could not be read"""
        try:
            block_hash = substrate.rpc_request("chain_getBlockHash", [])["result"]
            if block_hash:
                block_data = substrate.get_block(block_hash=block_hash)
                return block_data["header"]["number"]
            return None
        except Exception as e:
            logger.error(f"Error getting block number: {e}")
            # If it's an SSL error, try to refresh the connection
            if "SSL" in str(e) or "ssl" in str(e):
                logger.warning("SSL error detected in block number fetch, will attempt reconnection")
                # Mark for reconnection in the main loop
                if hasattr(self, '_hippius_ssl_error_count'):
                    self._hippius_ssl_error_count += 1
                else:
                    self._hippius_ssl_error_count = 1
            # None (not 0): returning 0 made blocks_since_last hugely negative and
            # silently suppressed submissions
            return None

    def update_uids_with_roles(self, uids, dividends):
        """Update UIDs with roles based on dividends"""
        updated_uids = []
        for uid in uids:
            updated_uid = uid.copy()
            if uid["id"] >= len(dividends):
                updated_uid["role"] = "Miner"
            elif uid["id"] == 0:
                updated_uid["role"] = "Validator"
            elif dividends[uid["id"]] > 0:
                updated_uid["role"] = "Validator"
            else:
                updated_uid["role"] = "Miner"
            updated_uids.append(updated_uid)
        return updated_uids

    def get_metagraph_snapshot(self):
        """Fetch UIDs and dividends for net_uid from ONE metagraph read.

        Both come from the same block, so dividends are always indexed by the
        same UIDs they are submitted with.

        Returns:
            (uids, dividends): uid dicts, and a uid-indexed list of u16 dividends.
            Both empty on failure.
        """
        try:
            # bittensor 11.x: typed metagraph under the subnets namespace (None if the subnet is missing)
            metagraph = self.bittensor_chain.subnets.metagraph(self.net_uid, commitments=False)
            if metagraph is None:
                logger.error(f"Subnet {self.net_uid} not found on {self.bittensor_network}")
                return [], []

            neurons = list(metagraph)
            if not neurons:
                logger.warning("Metagraph has no neurons")
                return [], []

            # Note: no axon filter — subnet 75 miners don't serve axons
            uids = [
                {
                    "address": n.hotkey,
                    "id": n.uid,
                    "role": "None",
                    "substrate_address": n.hotkey,
                }
                for n in neurons
            ]
            dividends = [0] * (max(n.uid for n in neurons) + 1)
            for n in neurons:
                dividends[n.uid] = dividend_to_u16(n.dividends)

            logger.info(f"Fetched {len(uids)} UIDs and {len(dividends)} dividends from Bittensor (block {metagraph.block})")
            return uids, dividends

        except Exception as e:
            logger.error(f"Error fetching metagraph: {e}")
            return [], []

    def submit_hotkeys_to_my_chain(self, hotkeys, dividends):
        """Submit hotkeys and dividends to your chain"""
        try:
            formatted_hotkeys = []
            for uid in hotkeys:
                pubkey_hex = ss58_decode(uid["address"])
                substrate_hex = ss58_decode(uid["substrate_address"])
                formatted_hotkeys.append({
                    "address": "0x" + pubkey_hex,
                    "id": uid["id"],
                    "role": uid["role"],
                    "substrate_address": "0x" + substrate_hex,
                })

            call = self.my_chain.compose_call(
                call_module="Metagraph",
                call_function="submit_hot_keys_info",
                call_params={
                    "hot_keys": formatted_hotkeys,
                    "dividends": dividends
                }
            )
            extrinsic = self.my_chain.create_signed_extrinsic(call=call, keypair=self.keypair)
            logger.info("Submitting extrinsic...")
            receipt = self.my_chain.submit_extrinsic(extrinsic, wait_for_inclusion=True)
            logger.info(f"Extrinsic submitted successfully: {receipt.extrinsic_hash}")
            if getattr(receipt, "is_success", False):
                logger.info("✅ Transaction included in block")
            else:
                logger.warning("⚠️ Transaction status unknown")
            return True

        except Exception as e:
            logger.error(f"Error submitting extrinsic: {e}")
            return False

    async def monitor_blocks(self):
        """Monitor blocks and submit tx every N blocks"""
        if not self.connect_chains():
            logger.error("Failed to connect to chains. Exiting.")
            return

        self.last_processed_block = self.get_block_number(self.my_chain)  # Hippius chain
        while self.last_processed_block is None:
            logger.warning("Could not read the Hippius block number, retrying in 10s...")
            await asyncio.sleep(10)
            self.last_processed_block = self.get_block_number(self.my_chain)
        logger.info(f"Starting monitoring from Hippius chain at block {self.last_processed_block}")
        logger.info(f"Will submit every {self.block_interval} Hippius blocks")
        
        # Option to submit immediately on startup (set to True to test connection)
        submit_immediately = False
        if submit_immediately:
            logger.info("🚀 Submitting immediately on startup...")
            uids, dividends = self.get_metagraph_snapshot()
            if uids and dividends:
                updated_uids = self.update_uids_with_roles(uids, dividends)
                if self.submit_hotkeys_to_my_chain(updated_uids, dividends):
                    logger.info("✅ Initial submission successful")
                else:
                    logger.warning("❌ Initial submission failed")
            else:
                logger.warning("⚠️ No data available for initial submission")

        while True:
            try:
                current_block = self.get_block_number(self.my_chain)
                
                # Check if we need to reconnect due to SSL errors
                if hasattr(self, '_hippius_ssl_error_count') and self._hippius_ssl_error_count > 3:
                    logger.warning("Multiple SSL errors detected on Hippius chain, attempting reconnection...")
                    try:
                        # Try to refresh just the Hippius connection first
                        logger.info("Refreshing Hippius chain connection...")
                        ssl_context = create_ssl_context()
                        try:
                            self.my_chain = SubstrateInterface(
                                url=self.my_chain_url,
                                use_remote_preset=True,
                                ss58_format=42,
                                ssl_context=ssl_context
                            )
                        except TypeError:
                            self.my_chain = SubstrateInterface(
                                url=self.my_chain_url,
                                use_remote_preset=True,
                                ss58_format=42
                            )
                        self._hippius_ssl_error_count = 0
                        logger.info("✅ Successfully reconnected to Hippius chain")
                        # Get fresh block number after reconnection
                        current_block = self.get_block_number(self.my_chain)
                    except Exception as reconnect_e:
                        logger.error(f"Failed to reconnect to Hippius chain: {reconnect_e}")
                        # Fall back to full reconnection
                        try:
                            self.connect_chains()
                            current_block = self.get_block_number(self.my_chain)
                        except Exception as full_reconnect_e:
                            logger.error(f"Full reconnection also failed: {full_reconnect_e}")
                            await asyncio.sleep(10)
                            continue

                if current_block is None:
                    # Hippius block unreadable this poll; skip rather than miscount
                    await asyncio.sleep(6)
                    continue

                blocks_since_last = current_block - self.last_processed_block
                
                # Log progress every 10 blocks
                if blocks_since_last % 10 == 0 and blocks_since_last > 0:
                    logger.info(f"Hippius block: {current_block}, Blocks since last submission: {blocks_since_last}/{self.block_interval}")

                if current_block - self.last_processed_block >= self.block_interval:
                    logger.info(f"\n--- Processing block {current_block} ---")
                    
                    # Try to fetch data from Bittensor with retry logic
                    max_retries = 3
                    retry_count = 0
                    uids = None
                    dividends = None
                    
                    while retry_count < max_retries and (not uids or not dividends):
                        retry_count += 1
                        logger.info(f"Attempt {retry_count}/{max_retries} to fetch data from Bittensor...")
                        
                        try:
                            uids, dividends = self.get_metagraph_snapshot()

                            if uids and dividends:
                                logger.info(f"✅ Successfully fetched {len(uids)} UIDs and {len(dividends)} dividends")
                                break
                            else:
                                logger.warning(f"⚠️ Attempt {retry_count}: Incomplete data (UIDs: {len(uids) if uids else 0}, Dividends: {len(dividends) if dividends else 0})")
                        except Exception as e:
                            logger.error(f"❌ Attempt {retry_count} failed: {e}")
                        
                        if retry_count < max_retries:
                            logger.info(f"🔄 Waiting 10 seconds before retry...")
                            await asyncio.sleep(10)
                    
                    if uids and dividends:
                        logger.info(f"Processing {len(uids)} UIDs and {len(dividends)} dividends")
                        updated_uids = self.update_uids_with_roles(uids, dividends)
                        validators = sum(1 for u in updated_uids if u["role"] == "Validator")
                        miners = sum(1 for u in updated_uids if u["role"] == "Miner")
                        logger.info(f"Roles: {validators} Validators, {miners} Miners")

                        if self.submit_hotkeys_to_my_chain(updated_uids, dividends):
                            self.last_processed_block = current_block
                            logger.info("✅ Successfully processed and submitted data")
                        else:
                            logger.warning("❌ Failed to submit data")
                    else:
                        logger.error("❌ Failed to fetch data from Bittensor after all retries")
                        logger.info("🔄 Will retry at next block interval")

                # Periodic connection health check (every 100 iterations = ~10 minutes)
                if hasattr(self, '_iteration_count'):
                    self._iteration_count += 1
                else:
                    self._iteration_count = 1
                
                if self._iteration_count % 100 == 0:
                    logger.info("🔄 Performing periodic connection health check...")
                    try:
                        # Test Hippius connection
                        test_block = self.get_block_number(self.my_chain)
                        if test_block is not None:
                            logger.info(f"✅ Hippius connection healthy (block: {test_block})")
                        else:
                            logger.warning("⚠️ Hippius connection may be unhealthy")
                    except Exception as health_e:
                        logger.warning(f"⚠️ Health check failed: {health_e}")
                
                await asyncio.sleep(6)

            except KeyboardInterrupt:
                logger.info("Stopping monitor...")
                break
            except Exception as e:
                logger.error(f"Error in monitor loop: {e}")
                logger.info("🔄 Attempting to reconnect...")
                self.connect_chains()
                await asyncio.sleep(10)

def main():
    import sys
    
    # Allow config file path as command line argument
    config_path = sys.argv[1] if len(sys.argv) > 1 else "config.yaml"
    
    monitor = StorageMonitor(config_path)
    try:
        asyncio.run(monitor.monitor_blocks())
    except Exception as e:
        logger.error(f"Fatal error: {e}")

if __name__ == "__main__":
    main()
