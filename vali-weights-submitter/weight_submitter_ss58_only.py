#!/usr/bin/env python3
"""
Bittensor Weight Submitter for Subnet 75 - SS58 Address Only Version
This version uses SS58 address for querying but requires local wallet for weight submission
"""

import asyncio
import logging
import yaml
import numpy as np
from typing import Dict, List, Tuple, Optional
import time

import bittensor as bt
from bittensor.core.extrinsics.set_weights import set_weights_extrinsic
from substrateinterface import SubstrateInterface, Keypair


class WeightSubmitterSS58:
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
        
        # Get validator SS58 address for querying
        if self.config['wallet'].get('use_ss58_address', False):
            self.validator_ss58 = self.config['wallet']['ss58_address']
            if not self.validator_ss58 or self.validator_ss58 == "5F...your_validator_ss58_address_here...":
                raise ValueError("Please set a valid SS58 address in config.yaml")
            self.logger.info(f"Using SS58 address for querying: {self.validator_ss58}")
            
            # Still need a wallet for weight submission, but we'll use it minimally
            self.wallet = bt.wallet(
                name=self.config['wallet']['name'],
                hotkey=self.config['wallet']['hotkey'],
                path=self.config['wallet']['path']
            )
            self.logger.info(f"Using local wallet for submission: {self.wallet.hotkey.ss58_address}")
        else:
            # Use local wallet for both querying and submission
            self.wallet = bt.wallet(
                name=self.config['wallet']['name'],
                hotkey=self.config['wallet']['hotkey'],
                path=self.config['wallet']['path']
            )
            self.validator_ss58 = self.wallet.hotkey.ss58_address
            self.logger.info(f"Using local wallet: {self.validator_ss58}")
        
        self.logger.info("Connections initialized successfully")
    
    async def fetch_metagraph(self) -> Dict[int, str]:
        """
        Step 1: Fetch the metagraph for subnet 75 and get all active hotkeys with their UIDs.
        
        Returns:
            Dict[int, str]: Mapping of UID to hotkey (SS58 address)
        """
        self.logger.info(f"Fetching metagraph for subnet {self.config['bittensor']['subnet_id']}...")
        
        try:
            # Get metagraph for the subnet
            metagraph = bt.metagraph(
                netuid=self.config['bittensor']['subnet_id'],
                subtensor=self.subtensor
            )
            
            # Get active neurons
            active_neurons = metagraph.neurons
            
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
                        self.logger.debug(f"No match found for SS58: {ss58_address}")
                
            except Exception as e:
                self.logger.warning(f"Error processing ranking entry: {e}")
                continue
        
        # Normalize weights to sum to 1.0
        if matched_weights:
            total_weight = sum(weight for _, weight in matched_weights)
            if total_weight > 0:
                matched_weights = [(uid, weight / total_weight) for uid, weight in matched_weights]
        
        self.logger.info(f"Matched {len(matched_weights)} rankings with UIDs")
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
        
        try:
            # Extract UIDs and weights
            uids = [uid for uid, _ in matched_weights]
            weights = [weight for _, weight in matched_weights]
            
            # Convert to numpy arrays as expected by the function
            uids_array = np.array(uids, dtype=np.int64)
            weights_array = np.array(weights, dtype=np.float32)
            
            # Submit weights using the bittensor function
            # Note: We still need a wallet for submission, but we can use the SS58 for querying
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
                # Log the submitted weights
                for uid, weight in matched_weights:
                    self.logger.info(f"UID {uid}: weight {weight:.6f}")
            else:
                self.logger.error("Failed to submit weights")
            
            return success
            
        except Exception as e:
            self.logger.error(f"Error submitting weights: {e}")
            return False
    
    async def run(self):
        """Main execution method that runs all steps."""
        try:
            # Initialize connections
            await self.initialize_connections()
            
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
            else:
                self.logger.warning("No weights to submit - no matches found")
                
        except Exception as e:
            self.logger.error(f"Error in main execution: {e}")
            raise
        finally:
            # Clean up connections
            if self.subtensor:
                self.subtensor.close()
            if self.hippius_interface:
                self.hippius_interface.close()


async def main():
    """Main entry point."""
    submitter = WeightSubmitterSS58()
    await submitter.run()


if __name__ == "__main__":
    asyncio.run(main())
