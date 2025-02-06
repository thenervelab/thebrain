TODO:

Frontend to upload files and get the hash





Todo:

- Pallet to track the file replications and shards
  - Track the file replications and shards
  - Ban and blacklist + removal

- Pallet for managing marketplace

- Pallet for managing worker nodes that will be used for reputation:
  - good and bad results
  - slashing
  - rewards

- Pallet authorization:
  - address linking ( miner and validators can link sub addresses to main account to ensure security)
  - all acl inside it, if a miner can write the chain, ....
  - blacklist
  - proof that validator ask weights stored in the chain to avoid weight copy



to be sure the miners don't fake and validator don't copy weight we will encrypt the weight with the public key of the validator
and the miner will encrypt the weight with the key and send it to the validator.
Same for miner we validator will encrypt the request with the public key of the miner




# TODO

- whitelist who can trigger benchmark ( validators in our case )







```
{
  "file_id": "file12345abc",
  "original_file_name": "example_document.pdf",
  "file_size_bytes": 10485760,
  "chunk_size_bytes": 524288,
  "total_chunks": 20,
  "merkle_root": "abc123efg456hij789klm012nop345qrstu678vwxyz",
  "chunks": [
    {
      "chunk_id": 1,
      "chunk_hash": "f1d2d2f924e986ac86fdf7b36c94bcdf32beec15a55a1199173",
      "ipfs_cid": "QmX1nYU8nJ42p8d8bcdf8f89d1bd8hgf6J7m",
      "merkle_path": [
        "hash_a1b2c3",
        "hash_d4e5f6",
        "hash_g7h8i9"
      ]
    },
    {
      "chunk_id": 2,
      "chunk_hash": "a0b1c2d3e4f5678901234567890abcdf123456789abcdef12345",
      "ipfs_cid": "QmX2jYU8pX88d4x6adfg7df4kft8sjJ7n",
      "merkle_path": [
        "hash_h1i2j3",
        "hash_k4l5m6",
        "hash_n7o8p9"
      ]
    }
  ],
  "registration_timestamp": "2024-10-21T15:30:00Z"
}
```

# Precompiles

1) IPFS Precompile Deployed at:
```0x0000000000000000000000000000000000000807```
it has functions to call_storage_request and read the pinned files of miners


2) RegistrationPrecompile Deployed at:
```0x0000000000000000000000000000000000000808```
it has functions to get registered nodes and there types