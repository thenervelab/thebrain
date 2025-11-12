#!/usr/bin/env python3
"""
Fix SSL certificate issues on macOS
"""

import ssl
import certifi
import os

def fix_ssl():
    print("Fixing SSL certificate issues...")
    
    # Set SSL certificate path
    os.environ['SSL_CERT_FILE'] = certifi.where()
    os.environ['SSL_CERT_DIR'] = certifi.where()
    
    print(f"SSL certificate file: {certifi.where()}")
    print("SSL environment variables set.")
    
    # Test SSL context
    try:
        context = ssl.create_default_context(cafile=certifi.where())
        print("✓ SSL context created successfully")
    except Exception as e:
        print(f"✗ SSL context creation failed: {e}")

if __name__ == "__main__":
    fix_ssl()
