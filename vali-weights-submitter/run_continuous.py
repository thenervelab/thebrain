#!/usr/bin/env python3
"""
Continuous weight submission script
Runs the weight submission process every 101 blocks (100 block intervals)
"""

import asyncio
import logging
import yaml
import time
import ssl
import certifi
import os
from datetime import datetime
from weight_submitter import WeightSubmitter

# Set SSL certificates for macOS
os.environ['SSL_CERT_FILE'] = certifi.where()
os.environ['SSL_CERT_DIR'] = certifi.where()


class ContinuousWeightSubmitter:
    def __init__(self, config_path: str = "config.yaml"):
        """Initialize the continuous weight submitter."""
        self.config = self._load_config(config_path)
        self._setup_logging()
        self.logger = logging.getLogger(__name__)
        self.running = False
        self.subtensor = None
        self.config_path = config_path
        self._weight_submitter = None

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
        import bittensor as bt
        
        self.subtensor = bt.subtensor(
            network=self.config['bittensor']['chain_endpoint']
        )
        self.logger.info("Initialized Bittensor connection for block monitoring")
    
    def calculate_next_submission_block(self, current_block: int, block_interval: int) -> int:
        """
        Calculate the next submission block using modulo arithmetic.
        
        Args:
            current_block: Current block number
            block_interval: Block interval for submissions
            
        Returns:
            int: Next submission block number
        """
        return ((current_block // block_interval) + 1) * block_interval
    
    async def get_current_block(self):
        """Get the current block number."""
        if not self.subtensor:
            await self.initialize_subtensor()
        
        try:
            block_number = self.subtensor.block
            return block_number
        except Exception as e:
            self.logger.error(f"Error getting current block: {e}")
            return None
    
    async def wait_for_next_submission_block(self, last_submission_block: int = None):
        """
        Wait until we reach the next submission block using modulo arithmetic.
        This ensures consistent submission intervals even if process is restarted.
        
        Args:
            last_submission_block: The block number of the last submission (optional)
        """
        block_interval = self.config['weights'].get('block_interval', 101)
        check_interval = self.config['weights'].get('block_check_interval', 12)
        
        while self.running:
            current_block = await self.get_current_block()
            if current_block is None:
                self.logger.warning("Could not get current block, waiting 30 seconds...")
                await asyncio.sleep(30)
                continue
            
            # Calculate if current block is a submission block using modulo
            # We want to submit when (current_block % block_interval) == 0
            # But we also want to ensure we don't submit too frequently
            if last_submission_block is not None:
                # If we have a last submission block, ensure we wait at least block_interval blocks
                min_target_block = last_submission_block + block_interval
                if current_block < min_target_block:
                    blocks_remaining = min_target_block - current_block
                    self.logger.info(f"Current block: {current_block}, min target: {min_target_block}, remaining: {blocks_remaining}")
                    await asyncio.sleep(check_interval)
                    continue
            
            # Check if current block is a submission block (modulo block_interval)
            if current_block % block_interval == 0:
                self.logger.info(f"Current block {current_block} is a submission block (modulo {block_interval})")
                return current_block
            
            # Calculate next submission block
            next_submission_block = self.calculate_next_submission_block(current_block, block_interval)
            blocks_remaining = next_submission_block - current_block
            
            self.logger.info(f"Current block: {current_block}, next submission: {next_submission_block}, remaining: {blocks_remaining}")
            
            # Wait for configured interval (approximately 1 block time)
            await asyncio.sleep(check_interval)
    
    async def run_continuous(self):
        """
        Run the weight submission process continuously every 101 blocks.
        """
        self.running = True
        block_interval = self.config['weights'].get('block_interval', 101)
        self.logger.info(f"Starting continuous weight submission every {block_interval} blocks")
        
        # Initialize Bittensor connection
        await self.initialize_subtensor()
        
        # Get current block as starting point
        current_block = await self.get_current_block()
        if current_block is None:
            self.logger.error("Could not get initial block number, exiting...")
            return
        
        block_interval = self.config['weights'].get('block_interval', 101)
        submission_count = 0
        last_submission_block = None
        
        # Check if current block is already a submission block
        if current_block % block_interval == 0:
            self.logger.info(f"Current block {current_block} is already a submission block, starting immediately")
        else:
            self.logger.info(f"Current block {current_block} is not a submission block, waiting for next one")
            current_block = await self.wait_for_next_submission_block()
        
        # Initialize reusable WeightSubmitter once to avoid metagraph/scalecodec memory leak
        if self._weight_submitter is None:
            self._weight_submitter = WeightSubmitter(self.config_path)
            await self._weight_submitter.initialize_connections()
            self.logger.info("Initialized reusable WeightSubmitter")

        while self.running:
            try:
                submission_count += 1
                start_time = datetime.now()
                self.logger.info(f"=== SUBMISSION #{submission_count} ===")
                self.logger.info(f"Starting weight submission at {start_time} (block {current_block})")

                success = await self._weight_submitter.run_submission()
                
                end_time = datetime.now()
                duration = (end_time - start_time).total_seconds()
                
                if success:
                    self.logger.info(f"✅ Weight submission #{submission_count} completed successfully in {duration:.2f} seconds")
                else:
                    self.logger.error(f"❌ Weight submission #{submission_count} failed after {duration:.2f} seconds")
                
                # Update last submission block
                last_submission_block = current_block
                
                # Wait for next submission block using modulo arithmetic
                if self.running:
                    self.logger.info(f"Waiting for next submission block (modulo {block_interval})...")
                    current_block = await self.wait_for_next_submission_block(last_submission_block)
                    
            except KeyboardInterrupt:
                self.logger.info("Received interrupt signal, stopping...")
                self.running = False
                break
            except Exception as e:
                self.logger.error(f"Error in continuous run: {e}")
                if self.running:
                    self.logger.info("Waiting 60 seconds before retry...")
                    await asyncio.sleep(60)
    
    def stop(self):
        """Stop the continuous submission process."""
        self.running = False
        self.logger.info("Stopping continuous weight submission...")
        if self.subtensor:
            self.subtensor.close()
        if self._weight_submitter:
            self._weight_submitter.cleanup()
            self._weight_submitter = None


async def main():
    """Main entry point for continuous operation."""
    import argparse
    
    parser = argparse.ArgumentParser(description="Continuous weight submission for Bittensor subnet 75")
    parser.add_argument("--config", type=str, default="config.yaml",
                       help="Configuration file path (default: config.yaml)")
    
    args = parser.parse_args()
    
    submitter = ContinuousWeightSubmitter(args.config)
    
    try:
        await submitter.run_continuous()
    except KeyboardInterrupt:
        submitter.stop()
        print("\nContinuous submission stopped by user")


if __name__ == "__main__":
    asyncio.run(main())
