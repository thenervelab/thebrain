#!/usr/bin/env python3
"""
Continuous weight submission script
Runs the weight submission process every `weights.block_interval` blocks (default 101)
"""

import argparse
import asyncio
import logging
import os
from datetime import datetime
from typing import Optional

import certifi
import yaml

# Set SSL certificates for macOS
os.environ['SSL_CERT_FILE'] = certifi.where()
os.environ['SSL_CERT_DIR'] = certifi.where()

import bittensor as bt

from weight_submitter import WeightSubmitter


class ContinuousWeightSubmitter:
    def __init__(self, config_path: str = "config.yaml", dry_run: bool = False):
        """Initialize the continuous weight submitter."""
        self.config_path = config_path
        self.dry_run = dry_run
        self.config = self._load_config(config_path)
        self._setup_logging()
        self.logger = logging.getLogger(__name__)
        self.running = False
        self.subtensor = None

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

    async def initialize_subtensor(self):
        """Initialize Bittensor connection for block monitoring."""
        # bittensor 11.x: blocking when used directly, connects lazily on first call
        self.subtensor = bt.Subtensor(network=self.config['bittensor']['chain_endpoint'])
        self.logger.info("Initialized Bittensor connection for block monitoring")

    def _reset_subtensor(self):
        """Drop the block-polling connection so the next poll reconnects."""
        if self.subtensor is not None:
            try:
                self.subtensor.close()
            except Exception:
                pass
            self.subtensor = None

    @staticmethod
    def calculate_next_submission_block(current_block: int, block_interval: int) -> int:
        """
        The next multiple of block_interval strictly after current_block.

        Args:
            current_block: Current block number
            block_interval: Block interval for submissions

        Returns:
            int: Next submission block number
        """
        return ((current_block // block_interval) + 1) * block_interval

    def first_target_block(self, current_block: int) -> int:
        """Block at which the first submission is due."""
        block_interval = self.config['weights'].get('block_interval', 101)
        if self.config['weights'].get('use_modulo_timing', True) and current_block % block_interval != 0:
            return self.calculate_next_submission_block(current_block, block_interval)
        return current_block

    def next_target_block(self, submitted_at_block: int) -> int:
        """Block at which the next submission is due, given when the last one ran."""
        block_interval = self.config['weights'].get('block_interval', 101)
        if self.config['weights'].get('use_modulo_timing', True):
            # Aligned to multiples of block_interval, consistent across restarts
            return self.calculate_next_submission_block(submitted_at_block, block_interval)
        return submitted_at_block + block_interval

    async def get_current_block(self) -> Optional[int]:
        """Get the current block number, or None if the chain could not be read."""
        if not self.subtensor:
            await self.initialize_subtensor()

        try:
            # bittensor 11.x: `block` is a property that reads the chain head
            return self.subtensor.block
        except Exception as e:
            self.logger.error(f"Error getting current block: {e}")
            self._reset_subtensor()
            return None

    async def run_continuous(self):
        """
        Run the weight submission process continuously every block_interval blocks.

        A submission runs as soon as the chain is at or past the target block, so
        a poll that lands a block late does not skip the whole window.
        """
        self.running = True
        block_interval = self.config['weights'].get('block_interval', 101)
        check_interval = self.config['weights'].get('block_check_interval', 12)
        self.logger.info(f"Starting continuous weight submission every {block_interval} blocks"
                         f"{' (DRY RUN)' if self.dry_run else ''}")

        submission_count = 0
        target_block = None

        while self.running:
            try:
                current_block = await self.get_current_block()
                if current_block is None:
                    self.logger.warning("Could not get current block, retrying in 30 seconds...")
                    await asyncio.sleep(30)
                    continue

                if target_block is None:
                    target_block = self.first_target_block(current_block)

                if current_block < target_block:
                    self.logger.info(f"Current block: {current_block}, next submission: {target_block}, "
                                     f"remaining: {target_block - current_block}")
                    await asyncio.sleep(check_interval)
                    continue

                submission_count += 1
                start_time = datetime.now()
                self.logger.info(f"=== SUBMISSION #{submission_count} ===")
                self.logger.info(f"Starting weight submission at {start_time} (block {current_block}, target {target_block})")

                submitter = WeightSubmitter(self.config_path, dry_run=self.dry_run)
                success = await submitter.run()

                duration = (datetime.now() - start_time).total_seconds()
                if success:
                    self.logger.info(f"✅ Weight submission #{submission_count} completed successfully in {duration:.2f} seconds")
                else:
                    self.logger.error(f"❌ Weight submission #{submission_count} failed after {duration:.2f} seconds")

                target_block = self.next_target_block(current_block)
                self.logger.info(f"Next submission at block {target_block}")

            except Exception as e:
                self.logger.error(f"Error in continuous run: {e}")
                if self.running:
                    self.logger.info("Waiting 60 seconds before retry...")
                    await asyncio.sleep(60)

    def stop(self):
        """Stop the continuous submission process."""
        self.running = False
        self.logger.info("Stopping continuous weight submission...")
        self._reset_subtensor()


async def main():
    """Main entry point for continuous operation."""
    parser = argparse.ArgumentParser(description="Continuous weight submission for Bittensor subnet 75")
    parser.add_argument("--config", type=str, default="config.yaml",
                        help="Configuration file path (default: config.yaml)")
    parser.add_argument("--dry-run", action="store_true",
                        help="Preflight each submission without signing or submitting")
    args = parser.parse_args()

    submitter = ContinuousWeightSubmitter(args.config, dry_run=args.dry_run)
    try:
        await submitter.run_continuous()
    finally:
        submitter.stop()


if __name__ == "__main__":
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        print("\nContinuous submission stopped by user")
