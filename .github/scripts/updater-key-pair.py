#!/usr/bin/env python3
"""Checks that the updater signing key and its password belong to the
public key, using only the Python standard library.

    python3 -I .github/scripts/updater-key-pair.py

Environment (never printed):
    TAURI_SIGNING_PRIVATE_KEY           base64 of the minisign secret key file
    TAURI_SIGNING_PRIVATE_KEY_PASSWORD  its password (may be empty)
    DECKPILOT_UPDATER_PUBKEY            base64 of the minisign public key file

The signing check runs this instead of the Tauri CLI or deckpilot-dev, so
that no dependency installation or build script runs while the key is in
the job: the minisign secret key is decrypted (scrypt, as libsodium's
crypto_pwhash_scryptsalsa208sha256), its BLAKE2b checksum proves the
password, and its key ID and Ed25519 public key must equal the public key's.
Only "match" or a reason is printed.
"""

import base64
import hashlib
import os
import struct
import sys


class Mismatch(Exception):
    pass


def payload(name):
    """The binary key line of the base64-encoded minisign key file in $name."""
    value = os.environ.get(name, "").strip()
    if not value:
        raise Mismatch(f"{name} is not set")
    try:
        text = base64.b64decode(value, validate=True).decode("utf-8")
        lines = [line for line in text.splitlines() if line.strip()]
        if len(lines) < 2 or not lines[0].startswith("untrusted comment:"):
            raise ValueError
        return base64.b64decode(lines[1].strip(), validate=True)
    except ValueError as error:
        raise Mismatch(f"{name} is not a base64-encoded minisign key file") from error


def scrypt_parameters(opslimit, memlimit):
    """N, r, p as libsodium's pickparams derives them."""
    opslimit = max(opslimit, 32768)
    r = 8
    if opslimit < memlimit // 32:
        p = 1
        max_n = opslimit // (r * 4)
        n_log2 = next(n for n in range(1, 63) if (1 << n) > max_n // 2)
    else:
        max_n = memlimit // (r * 128)
        n_log2 = next(n for n in range(1, 63) if (1 << n) > max_n // 2)
        max_rp = min((opslimit // 4) // (1 << n_log2), 0x3FFFFFFF)
        p = max(max_rp // r, 1)
    return 1 << n_log2, r, p


def secret_key(data, password):
    """(key ID, Ed25519 public key) of a minisign secret key."""
    if len(data) != 158:
        raise Mismatch("the secret key has an unexpected length")
    sig_alg, kdf_alg, chk_alg = data[0:2], data[2:4], data[4:6]
    salt = data[6:38]
    opslimit, memlimit = struct.unpack("<QQ", data[38:54])
    keynum_sk = data[54:158]
    if sig_alg != b"Ed" or chk_alg != b"B2":
        raise Mismatch("the secret key is not an Ed25519 minisign key")
    if kdf_alg == b"Sc":
        n, r, p = scrypt_parameters(opslimit, memlimit)
        stream = hashlib.scrypt(
            password.encode("utf-8"),
            salt=salt,
            n=n,
            r=r,
            p=p,
            maxmem=129 * r * n * p + (64 << 20),
            dklen=len(keynum_sk),
        )
        keynum_sk = bytes(a ^ b for a, b in zip(keynum_sk, stream))
    elif kdf_alg != b"\0\0":
        raise Mismatch("the secret key uses an unknown key derivation")
    keynum, sk, checksum = keynum_sk[0:8], keynum_sk[8:72], keynum_sk[72:104]
    expected = hashlib.blake2b(sig_alg + keynum + sk, digest_size=32).digest()
    if checksum != expected:
        raise Mismatch("the password does not decrypt the secret key (wrong password or key)")
    return keynum, sk[32:64]


def main():
    try:
        public = payload("DECKPILOT_UPDATER_PUBKEY")
        if len(public) != 42 or public[0:2] != b"Ed":
            raise Mismatch("DECKPILOT_UPDATER_PUBKEY is not an Ed25519 minisign public key")
        keynum, pk = secret_key(
            payload("TAURI_SIGNING_PRIVATE_KEY"),
            os.environ.get("TAURI_SIGNING_PRIVATE_KEY_PASSWORD", ""),
        )
        if keynum != public[2:10] or pk != public[10:42]:
            raise Mismatch("the signing key does not belong to DECKPILOT_UPDATER_PUBKEY")
    except Mismatch as error:
        print(f"::error::{error}")
        return 1
    print("match: the signing key and its password belong to DECKPILOT_UPDATER_PUBKEY")
    return 0


if __name__ == "__main__":
    sys.exit(main())
