#!/usr/bin/env python3
"""
Bittensor Weight Submitter for Subnet 75
Fetches rankings from Hippius chain and submits weights to Bittensor
"""

import asyncio
import logging
import yaml
import numpy as np
from typing import Dict, List, Tuple, Optional
import time
import ssl
import certifi
import os

# Fix SSL certificate issues on macOS
os.environ['SSL_CERT_FILE'] = certifi.where()
os.environ['SSL_CERT_DIR'] = certifi.where()

import bittensor as bt
from bittensor.core.extrinsics.set_weights import set_weights_extrinsic
from substrateinterface import SubstrateInterface, Keypair


class WeightSubmitter:
    def __init__(self, config_path: str = "config.yaml"):
        """Initialize the weight submitter with configuration."""
        self.config = self._load_config(config_path)
        self._setup_logging()
        self.logger = logging.getLogger(__name__)
        
        # Initialize connections
        self.subtensor = None
        self.hippius_interface = None
        self.wallet = None
        self.validator_ss58 = None
        self._metagraph = None
        
    def _load_config(self, config_path: str) -> dict:
        """Load configuration from YAML file."""
        try:
            with open(config_path, 'r') as file:
                return yaml.safe_load(file)
        except FileNotFoundError:
            raise FileNotFoundError(f"Configuration file {config_path} not found")
        except yaml.YAMLError as e:
            raise ValueError(f"Error parsing configuration file: {e}")
    
    def _setup_logging(self):
        """Setup logging configuration."""
        logging.basicConfig(
            level=getattr(logging, self.config['logging']['level']),
            format=self.config['logging']['format']
        )
    
    async def initialize_connections(self):
        """Initialize connections to Bittensor and Hippius chain."""
        self.logger.info("Initializing connections...")
        
        # Initialize Bittensor subtensor
        self.subtensor = bt.subtensor(
            network=self.config['bittensor']['chain_endpoint']
        )
        
        # Initialize Hippius chain interface
        self.hippius_interface = SubstrateInterface(
            url=self.config['hippius']['rpc_endpoint'],
            ss58_format=42,  # Bittensor uses ss58 format 42
            type_registry_preset="substrate-node-template"
        )
        
        # Initialize wallet (required for weight submission)
        # Create wallet from mnemonic using substrate-interface
        if self.config['wallet'].get('use_mnemonic', False):
            # Use mnemonic phrase from environment variable
            mnemonic = os.getenv('BITTENSOR_MNEMONIC')
            if not mnemonic:
                raise ValueError("BITTENSOR_MNEMONIC environment variable is required when use_mnemonic is true")
            
            # Create keypair from mnemonic using substrate-interface
            from substrateinterface import Keypair
            keypair = Keypair.create_from_mnemonic(mnemonic, ss58_format=42)
            
            # Create a minimal wallet-like object that works with our substrate-interface approach
            class MnemonicWallet:
                def __init__(self, keypair):
                    self.hotkey = type('Hotkey', (), {
                        'ss58_address': keypair.ss58_address,
                        'keypair': keypair
                    })()
                    self.keypair = keypair
                    self.coldkey = type('Coldkey', (), {
                        'ss58_address': keypair.ss58_address,
                        'keypair': keypair
                    })()
            
            self.wallet = MnemonicWallet(keypair)
            self.logger.info(f"Created wallet from mnemonic: {self.wallet.hotkey.ss58_address}")
        else:
            # Use local wallet files
            try:
                self.wallet = bt.wallet(
                    name=self.config['wallet']['name'],
                    hotkey=self.config['wallet']['hotkey'],
                    path=self.config['wallet']['path']
                )
                self.logger.info(f"Using local wallet: {self.wallet.hotkey.ss58_address}")
            except Exception as e:
                self.logger.error(f"Failed to load wallet: {e}")
                self.logger.info("Please create a wallet first using: python setup_wallet.py")
                raise
        
        # Set validator SS58 address for querying
        if self.config['wallet'].get('use_ss58_address', False):
            # Use SS58 address from config for querying
            self.validator_ss58 = self.config['wallet']['ss58_address']
            if not self.validator_ss58 or self.validator_ss58 == "5F...your_validator_ss58_address_here...":
                raise ValueError("Please set a valid SS58 address in config.yaml")
            self.logger.info(f"Using SS58 address for querying: {self.validator_ss58}")
        else:
            # Use wallet's SS58 address for both querying and submission
            self.validator_ss58 = self.wallet.hotkey.ss58_address
            self.logger.info(f"Using local wallet for both querying and submission: {self.validator_ss58}")
        
        self.logger.info("Connections initialized successfully")
    
    async def fetch_metagraph(self) -> Dict[int, str]:
        """
        Step 1: Fetch the metagraph for subnet 75 and get all active hotkeys with their UIDs.
        
        Returns:
            Dict[int, str]: Mapping of UID to hotkey (SS58 address)
        """
        self.logger.info(f"Fetching metagraph for subnet {self.config['bittensor']['subnet_id']}...")
        
        try:
            # Create metagraph once, then sync to refresh (avoids scalecodec type registration leak)
            if self._metagraph is None:
                self._metagraph = bt.metagraph(
                    netuid=self.config['bittensor']['subnet_id'],
                    subtensor=self.subtensor
                )
            else:
                self._metagraph.sync(subtensor=self.subtensor)

            active_neurons = self._metagraph.neurons
            
            # Create mapping of UID to hotkey
            uid_to_hotkey = {}
            for uid, neuron in enumerate(active_neurons):
                if neuron.axon_info.ip != 0:  # Active neuron
                    uid_to_hotkey[uid] = neuron.axon_info.hotkey
            
            self.logger.info(f"Found {len(uid_to_hotkey)} active neurons in subnet {self.config['bittensor']['subnet_id']}")
            return uid_to_hotkey
            
        except Exception as e:
            self.logger.error(f"Error fetching metagraph: {e}")
            raise
    
    async def fetch_hippius_rankings(self) -> List[Dict]:
        """
        Step 2: Fetch the ranking pallet from Hippius chain.
        
        Returns:
            List[Dict]: List of ranked nodes from Hippius chain
        """
        self.logger.info("Fetching rankings from Hippius chain...")
        
        try:
            # Query the ranking storage
            result = self.hippius_interface.query(
                module='RankingStorage',
                storage_function='RankedList'
            )
            
            if result and hasattr(result, 'value'):
                rankings = result.value
                self.logger.info(f"Retrieved {len(rankings)} rankings from Hippius chain")
                return rankings
            else:
                self.logger.warning("No rankings found or empty result")
                return []
                
        except Exception as e:
            self.logger.error(f"Error fetching Hippius rankings: {e}")
            raise
    
    def match_rankings_with_uids(self, uid_to_hotkey: Dict[int, str], 
                                hippius_rankings: List[Dict]) -> List[Tuple[int, float]]:
        """
        Step 3: Match SS58 addresses from rankings with UIDs of the Bittensor metagraph.
        
        Args:
            uid_to_hotkey: Mapping of UID to hotkey (SS58 address)
            hippius_rankings: List of ranked nodes from Hippius chain
            
        Returns:
            List[Tuple[int, float]]: List of (UID, weight) tuples for matched accounts
        """
        self.logger.info("Matching rankings with UIDs...")
        
        matched_weights = []
        unmatched_count = 0
        hotkey_to_uid = {hotkey: uid for uid, hotkey in uid_to_hotkey.items()}
        
        for ranking in hippius_rankings:
            try:
                # Extract SS58 address and ranking score from the ranking data
                # Based on the actual Hippius data structure
                ss58_address = ranking.get('node_ss58_address')
                ranking_score = ranking.get('weight')
                
                if ss58_address and ranking_score is not None:
                    # Check if this SS58 address exists in our metagraph
                    if ss58_address in hotkey_to_uid:
                        uid = hotkey_to_uid[ss58_address]
                        # Normalize the weight (assuming ranking_score is a positive number)
                        weight = float(ranking_score)
                        matched_weights.append((uid, weight))
                        self.logger.debug(f"Matched UID {uid} with weight {weight}")
                    else:
                        unmatched_count += 1
                        self.logger.debug(f"No match found for SS58: {ss58_address}")
                
            except Exception as e:
                self.logger.warning(f"Error processing ranking entry: {e}")
                continue
        
        # Log matching statistics
        self.logger.info(f"Matched {len(matched_weights)} rankings with UIDs")
        if unmatched_count > 0:
            self.logger.info(f"Unmatched {unmatched_count} rankings (SS58 addresses not found in metagraph)")
        
        # Log some sample matches for verification
        if matched_weights:
            self.logger.info("Sample matches:")
            for i, (uid, weight) in enumerate(matched_weights[:5]):  # Show first 5
                ss58_address = next(addr for addr, u in hotkey_to_uid.items() if u == uid)
                self.logger.info(f"  UID {uid:3d} -> SS58 {ss58_address[:20]}... -> weight {weight:.6f}")
            if len(matched_weights) > 5:
                self.logger.info(f"  ... and {len(matched_weights) - 5} more matches")
        
        # Normalize weights to u16 range (0-65535) for each individual weight
        if matched_weights:
            total_weight = sum(weight for _, weight in matched_weights)
            if total_weight > 0:
                self.logger.info(f"Normalizing weights (original sum: {total_weight:.6f})")
                # Scale each weight to u16 range (0-65535)
                max_u16 = 65535
                matched_weights = [(uid, int((weight / total_weight) * max_u16)) for uid, weight in matched_weights]
                self.logger.info(f"Normalized weight sum: {sum(weight for _, weight in matched_weights)}")
        return matched_weights
    
    async def submit_weights(self, matched_weights: List[Tuple[int, float]]) -> bool:
        """
        Step 4: Submit weights to Bittensor using the set_weights_extrinsic function.
        
        Args:
            matched_weights: List of (UID, weight) tuples
            
        Returns:
            bool: True if successful, False otherwise
        """
        if not matched_weights:
            self.logger.warning("No weights to submit")
            return False
        
        self.logger.info(f"Submitting weights for {len(matched_weights)} neurons...")
        
        # Log the weights that will be submitted
        self.logger.info("=== WEIGHTS TO BE SUBMITTED ===")
        total_weight = sum(weight for _, weight in matched_weights)
        self.logger.info(f"Total neurons: {len(matched_weights)}")
        self.logger.info(f"Total weight sum: {total_weight}")
        
        # Check if we should log all individual weights
        log_all_weights = self.config['logging'].get('log_all_weights', True)
        max_weights_to_log = self.config['logging'].get('max_weights_to_log', 50)
        
        if log_all_weights:
            self.logger.info("Individual weights:")
            weights_to_log = matched_weights
            if max_weights_to_log > 0 and len(matched_weights) > max_weights_to_log:
                weights_to_log = matched_weights[:max_weights_to_log]
                self.logger.info(f"  (showing first {max_weights_to_log} weights out of {len(matched_weights)})")
            
            for uid, weight in weights_to_log:
                percentage = (weight / total_weight * 100) if total_weight > 0 else 0
                self.logger.info(f"  UID {uid:3d}: weight {weight} ({percentage:.2f}%)")
            
            if max_weights_to_log > 0 and len(matched_weights) > max_weights_to_log:
                self.logger.info(f"  ... and {len(matched_weights) - max_weights_to_log} more weights")
        else:
            self.logger.info("Individual weights logging disabled (set log_all_weights: true to enable)")
        
        self.logger.info("=== END WEIGHTS TO BE SUBMITTED ===")
        
        try:
            # Extract UIDs and weights
            uids = [uid for uid, _ in matched_weights]
            weights = [weight for _, weight in matched_weights]
            
            # Convert to numpy arrays as expected by the function
            uids_array = np.array(uids, dtype=np.int64)
            weights_array = np.array(weights, dtype=np.uint16)  # Use uint16 for individual weights
            
            # Submit weights using the appropriate method based on wallet type
            if hasattr(self.wallet, 'keypair') and hasattr(self.wallet.keypair, 'private_key'):
                # Mnemonic wallet - use substrate-interface directly
                self.logger.info(f"Submitting weights using mnemonic wallet: {self.wallet.hotkey.ss58_address}")
                
                # Use substrate-interface to submit the transaction
                from substrateinterface import SubstrateInterface
                
                # Create substrate interface for Bittensor
                substrate = SubstrateInterface(
                    url=self.config['bittensor']['rpc_endpoint'],
                    ss58_format=42,
                    type_registry_preset="substrate-node-template"
                )
                
                # Prepare the call
                call = substrate.compose_call(
                    call_module='SubtensorModule',
                    call_function='set_weights',
                    call_params={
                        'netuid': self.config['bittensor']['subnet_id'],
                        'dests': uids_array.tolist(),
                        'weights': weights_array.tolist(),
                        'version_key': self.config['weights']['version_key']
                    }
                )
                
                # Submit the transaction
                extrinsic = substrate.create_signed_extrinsic(
                    call=call,
                    keypair=self.wallet.keypair
                )
                
                # Submit and wait
                result = substrate.submit_extrinsic(
                    extrinsic=extrinsic,
                    wait_for_inclusion=self.config['weights']['wait_for_inclusion'],
                    wait_for_finalization=self.config['weights']['wait_for_finalization']
                )
                
                success = result.is_success
                
            else:
                # Local wallet - use Bittensor's set_weights_extrinsic
                self.logger.info(f"Submitting weights using local wallet: {self.wallet.hotkey.ss58_address}")
                
                success = set_weights_extrinsic(
                    subtensor=self.subtensor,
                    wallet=self.wallet,
                    netuid=self.config['bittensor']['subnet_id'],
                    uids=uids_array,
                    weights=weights_array,
                    version_key=self.config['weights']['version_key'],
                    wait_for_inclusion=self.config['weights']['wait_for_inclusion'],
                    wait_for_finalization=self.config['weights']['wait_for_finalization'],
                    period=self.config['weights']['period']
                )
            
            if success:
                self.logger.info("Weights submitted successfully")
                # Log the submitted weights in detail
                self.logger.info("=== SUBMITTED WEIGHTS DETAIL ===")
                total_weight = sum(weight for _, weight in matched_weights)
                self.logger.info(f"Total neurons: {len(matched_weights)}")
                self.logger.info(f"Total weight sum: {total_weight:.6f}")
                
                # Check if we should log all individual weights
                log_all_weights = self.config['logging'].get('log_all_weights', True)
                max_weights_to_log = self.config['logging'].get('max_weights_to_log', 50)
                
                if log_all_weights:
                    self.logger.info("Individual weights:")
                    weights_to_log = matched_weights
                    if max_weights_to_log > 0 and len(matched_weights) > max_weights_to_log:
                        weights_to_log = matched_weights[:max_weights_to_log]
                        self.logger.info(f"  (showing first {max_weights_to_log} weights out of {len(matched_weights)})")
                    
                    for uid, weight in weights_to_log:
                        percentage = (weight / total_weight * 100) if total_weight > 0 else 0
                        self.logger.info(f"  UID {uid:3d}: weight {weight} ({percentage:.2f}%)")
                    
                    if max_weights_to_log > 0 and len(matched_weights) > max_weights_to_log:
                        self.logger.info(f"  ... and {len(matched_weights) - max_weights_to_log} more weights")
                else:
                    self.logger.info("Individual weights logging disabled (set log_all_weights: true to enable)")
                
                self.logger.info("=== END WEIGHTS DETAIL ===")
            else:
                self.logger.error("Failed to submit weights")
                # Still log the weights that were attempted
                self.logger.info("=== ATTEMPTED WEIGHTS DETAIL ===")
                total_weight = sum(weight for _, weight in matched_weights)
                self.logger.info(f"Total neurons: {len(matched_weights)}")
                self.logger.info(f"Total weight sum: {total_weight}")
                
                # Check if we should log all individual weights
                log_all_weights = self.config['logging'].get('log_all_weights', True)
                max_weights_to_log = self.config['logging'].get('max_weights_to_log', 50)
                
                if log_all_weights:
                    self.logger.info("Individual weights:")
                    weights_to_log = matched_weights
                    if max_weights_to_log > 0 and len(matched_weights) > max_weights_to_log:
                        weights_to_log = matched_weights[:max_weights_to_log]
                        self.logger.info(f"  (showing first {max_weights_to_log} weights out of {len(matched_weights)})")
                    
                    for uid, weight in weights_to_log:
                        percentage = (weight / total_weight * 100) if total_weight > 0 else 0
                        self.logger.info(f"  UID {uid:3d}: weight {weight} ({percentage:.2f}%)")
                    
                    if max_weights_to_log > 0 and len(matched_weights) > max_weights_to_log:
                        self.logger.info(f"  ... and {len(matched_weights) - max_weights_to_log} more weights")
                else:
                    self.logger.info("Individual weights logging disabled (set log_all_weights: true to enable)")
                
                self.logger.info("=== END WEIGHTS DETAIL ===")
            
            return success
            
        except Exception as e:
            self.logger.error(f"Error submitting weights: {e}")
            return False
    
    async def run_submission(self) -> bool:
        """Run weight submission without reinitializing connections.

        Designed for repeated calls in a loop. Assumes initialize_connections()
        was already called. Does not close connections — caller manages lifecycle.
        """
        # Step 1: Fetch metagraph
        uid_to_hotkey = await self.fetch_metagraph()

        # Step 2: Fetch Hippius rankings
        hippius_rankings = await self.fetch_hippius_rankings()

        # Step 3: Match rankings with UIDs
        matched_weights = self.match_rankings_with_uids(uid_to_hotkey, hippius_rankings)

        # Step 4: Submit weights
        if matched_weights:
            success = await self.submit_weights(matched_weights)
            if success:
                self.logger.info("Weight submission completed successfully")
            else:
                self.logger.error("Weight submission failed")
            return success
        else:
            self.logger.warning("No weights to submit - no matches found")
            return False

    def cleanup(self):
        """Clean up connections. Call when done using this instance."""
        self.logger.info("Cleaning up WeightSubmitter connections...")
        if self.subtensor:
            self.subtensor.close()
            self.subtensor = None
        if self.hippius_interface:
            self.hippius_interface.close()
            self.hippius_interface = None
        self._metagraph = None

    async def run(self):
        """Main execution method that runs all steps (one-shot mode)."""
        try:
            await self.initialize_connections()
            return await self.run_submission()
        except Exception as e:
            self.logger.error(f"Error in main execution: {e}")
            return False
        finally:
            self.cleanup()


async def main():
    """Main entry point."""
    submitter = WeightSubmitter()
    await submitter.run()


if __name__ == "__main__":
    asyncio.run(main())
