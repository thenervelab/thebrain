#!/usr/bin/env python3
"""
Bittensor Weight Submitter for Subnet 75
Fetches rankings from Hippius chain and submits weights to Bittensor
"""

import argparse
import asyncio
import logging
import os
import sys
from typing import Dict, List, Tuple

import certifi
import yaml

# Fix SSL certificate issues on macOS
os.environ['SSL_CERT_FILE'] = certifi.where()
os.environ['SSL_CERT_DIR'] = certifi.where()

import bittensor as bt
from substrateinterface import SubstrateInterface

from wallet_utils import load_signing_wallet


class WeightSubmitter:
    def __init__(self, config_path: str = "config.yaml", dry_run: bool = False):
        """Initialize the weight submitter with configuration."""
        self.config = self._load_config(config_path)
        self.dry_run = dry_run
        self._setup_logging()
        self.logger = logging.getLogger(__name__)

        # Initialize connections
        self.subtensor = None
        self.hippius_interface = None
        self.wallet = None
        self.hotkey_ss58 = None

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

        # One SDK client for metagraph reads and weight submission. bittensor 11.x:
        # blocking when used directly, connects lazily on first call.
        self.subtensor = bt.Subtensor(network=self.config['bittensor']['chain_endpoint'])

        # Initialize Hippius chain interface
        self.hippius_interface = SubstrateInterface(
            url=self.config['hippius']['rpc_endpoint'],
            ss58_format=42,  # Bittensor uses ss58 format 42
            type_registry_preset="substrate-node-template"
        )

        # Signing wallet: mnemonic (BITTENSOR_MNEMONIC) or on-disk btcli wallet
        self.wallet, self.hotkey_ss58 = load_signing_wallet(self.config['wallet'])
        source = "mnemonic" if self.config['wallet'].get('use_mnemonic', False) else "local wallet files"
        self.logger.info(f"Signing with hotkey {self.hotkey_ss58} ({source})")

        self.logger.info("Connections initialized successfully")

    async def fetch_metagraph(self) -> Dict[int, str]:
        """
        Step 1: Fetch the metagraph for subnet 75 and get all active hotkeys with their UIDs.

        Returns:
            Dict[int, str]: Mapping of UID to hotkey (SS58 address)
        """
        netuid = self.config['bittensor']['subnet_id']
        self.logger.info(f"Fetching metagraph for subnet {netuid}...")

        try:
            metagraph = self.subtensor.subnets.metagraph(netuid, commitments=False)
            if metagraph is None:
                raise RuntimeError(f"Subnet {netuid} does not exist on {self.subtensor.network}")

            # Create mapping of UID to hotkey
            # Note: no axon filter — subnet 75 miners don't serve axons
            uid_to_hotkey = {neuron.uid: neuron.hotkey for neuron in metagraph}

            self.logger.info(f"Found {len(uid_to_hotkey)} neurons in subnet {netuid}")
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
            List[Tuple[int, float]]: (UID, weight) pairs sorted by UID, weights normalized
            to sum to 1.0. The SDK's SetWeights intent does the subnet-specific
            clipping and u16 quantization at submission time.
        """
        self.logger.info("Matching rankings with UIDs...")

        weights_by_uid: Dict[int, float] = {}
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
                        if uid in weights_by_uid:
                            self.logger.warning(f"Duplicate ranking for UID {uid} ({ss58_address}); summing weights")
                        weights_by_uid[uid] = weights_by_uid.get(uid, 0.0) + float(ranking_score)
                        self.logger.debug(f"Matched UID {uid} with weight {ranking_score}")
                    else:
                        unmatched_count += 1
                        self.logger.debug(f"No match found for SS58: {ss58_address}")

            except Exception as e:
                self.logger.warning(f"Error processing ranking entry: {e}")
                continue

        matched_weights = sorted(weights_by_uid.items())

        # Log matching statistics
        self.logger.info(f"Matched {len(matched_weights)} rankings with UIDs")
        if unmatched_count > 0:
            self.logger.info(f"Unmatched {unmatched_count} rankings (SS58 addresses not found in metagraph)")

        # Log some sample matches for verification
        if matched_weights:
            self.logger.info("Sample matches:")
            for uid, weight in matched_weights[:5]:
                self.logger.info(f"  UID {uid:3d} -> SS58 {uid_to_hotkey[uid][:20]}... -> weight {weight:.6f}")
            if len(matched_weights) > 5:
                self.logger.info(f"  ... and {len(matched_weights) - 5} more matches")

        # Normalize weights to sum to 1.0
        total_weight = sum(weight for _, weight in matched_weights)
        if matched_weights and total_weight <= 0:
            self.logger.warning("All matched ranking weights are zero; nothing to submit")
            return []
        if matched_weights:
            self.logger.info(f"Normalizing weights (original sum: {total_weight:.6f})")
            matched_weights = [(uid, weight / total_weight) for uid, weight in matched_weights]
        return matched_weights

    def _log_weights(self, title: str, weights: List[Tuple[int, float]]):
        """Log a weight vector, honouring the logging.log_all_weights / max_weights_to_log options."""
        self.logger.info(f"=== {title} ===")
        self.logger.info(f"Total neurons: {len(weights)}")

        if not self.config['logging'].get('log_all_weights', True):
            self.logger.info("Individual weights logging disabled (set log_all_weights: true to enable)")
        else:
            max_weights_to_log = self.config['logging'].get('max_weights_to_log', 50)
            shown = weights
            if max_weights_to_log > 0 and len(weights) > max_weights_to_log:
                shown = weights[:max_weights_to_log]
                self.logger.info(f"  (showing first {max_weights_to_log} weights out of {len(weights)})")
            self.logger.info("Individual weights:")
            for uid, weight in shown:
                self.logger.info(f"  UID {uid:3d}: weight {weight:.6f} ({weight * 100:.2f}%)")
            if len(shown) < len(weights):
                self.logger.info(f"  ... and {len(weights) - len(shown)} more weights")

        self.logger.info(f"=== END {title} ===")

    async def submit_weights(self, matched_weights: List[Tuple[int, float]]) -> bool:
        """
        Step 4: Submit weights to Bittensor through the SDK's SetWeights intent.

        SetWeights preflights registration and the rate limit, conforms the
        weights to the subnet hyperparameters, and picks plaintext or
        commit-reveal submission automatically.

        Args:
            matched_weights: List of (UID, weight) tuples

        Returns:
            bool: True if successful (or, with --dry-run, if the preflight passed)
        """
        if not matched_weights:
            self.logger.warning("No weights to submit")
            return False

        intent = bt.SetWeights(
            netuid=self.config['bittensor']['subnet_id'],
            weights=dict(matched_weights),
            version_key=self.config['weights']['version_key'],
        )
        self._log_weights("WEIGHTS TO BE SUBMITTED", matched_weights)

        weights_cfg = self.config['weights']
        try:
            if self.dry_run:
                plan = self.subtensor.plan(intent, self.wallet, period=weights_cfg['period'])
                self.logger.info(f"DRY RUN: preflight passed, nothing signed or submitted. {plan.summary}")
                for effect in plan.effects:
                    self.logger.info(f"DRY RUN: {effect}")
                for warning in plan.warnings:
                    self.logger.warning(f"DRY RUN: {warning}")
                return True

            self.logger.info(f"Submitting weights for {len(matched_weights)} neurons with hotkey {self.hotkey_ss58}...")
            result = self.subtensor.execute(
                intent,
                self.wallet,
                wait_for_inclusion=weights_cfg['wait_for_inclusion'],
                wait_for_finalization=weights_cfg['wait_for_finalization'],
                period=weights_cfg['period'],
                retries=weights_cfg.get('retries', 2),
            )
        except bt.ChainError as e:
            # Raised by the preflight (e.g. not registered, rate limited)
            self.logger.error(f"SetWeights rejected: {e.name or e.code.name} - {e.message} | {e.remediation}")
            return False
        except Exception as e:
            self.logger.error(f"Error submitting weights: {e}")
            return False

        if result.success:
            where = result.extrinsic_id or result.block_hash or "submitted"
            self.logger.info(f"Weights submitted successfully ({where})")
            return True

        if result.error is not None:
            self.logger.error(
                f"SetWeights failed: {result.error.name} - {result.error.message} | {result.error.remediation}"
            )
        else:
            self.logger.error(f"SetWeights failed: {result.message}")
        return False

    def close(self):
        """Close chain connections (safe to call more than once)."""
        if self.subtensor is not None:
            try:
                self.subtensor.close()
            except Exception as e:
                self.logger.debug(f"Error closing subtensor: {e}")
            self.subtensor = None
        if self.hippius_interface is not None:
            try:
                self.hippius_interface.close()
            except Exception as e:
                self.logger.debug(f"Error closing Hippius connection: {e}")
            self.hippius_interface = None

    async def run(self) -> bool:
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
                return success
            else:
                self.logger.warning("No weights to submit - no matches found")
                return False

        except Exception as e:
            self.logger.error(f"Error in main execution: {e}")
            return False
        finally:
            self.close()


def parse_args(argv=None):
    parser = argparse.ArgumentParser(description="Submit subnet 75 weights from Hippius rankings (single pass)")
    parser.add_argument("--config", type=str, default="config.yaml",
                        help="Configuration file path (default: config.yaml)")
    parser.add_argument("--dry-run", action="store_true",
                        help="Build and preflight the SetWeights call without signing or submitting")
    return parser.parse_args(argv)


async def main(argv=None) -> int:
    """Main entry point. Returns a process exit code (0 on success)."""
    args = parse_args(argv)
    submitter = WeightSubmitter(args.config, dry_run=args.dry_run)
    return 0 if await submitter.run() else 1


if __name__ == "__main__":
    sys.exit(asyncio.run(main()))
