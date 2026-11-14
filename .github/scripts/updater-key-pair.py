#!/usr/bin/env python3
"""Checks the updater signing key pair, using only the Python standard
library.

    python3 -I .github/scripts/updater-key-pair.py [--pinned FILE [--require-variable]] [--public-only]

Environment (never printed):
    TAURI_SIGNING_PRIVATE_KEY           base64 of the minisign secret key file
    TAURI_SIGNING_PRIVATE_KEY_PASSWORD  its password (may be empty)
    DECKPILOT_UPDATER_PUBKEY            base64 of the minisign public key file

--pinned FILE: the public key is the one committed in FILE (base64 of the
minisign public key file, .github/updater.pub). DECKPILOT_UPDATER_PUBKEY,
when set, must be the same key; with --require-variable it must be set.
--public-only: check the public key alone; no secret key is read.

Without --public-only, the minisign secret key is decrypted (scrypt, as
libsodium's crypto_pwhash_scryptsalsa208sha256), its BLAKE2b checksum proves
the password, and its key ID and Ed25519 public key must equal the public
key's. The release workflow runs this in the protected environment, so no
dependency installation or build script runs before it while the key is in
the job. Only "match", "public key …" or a reason is printed.
"""

import argparse
import base64
import hashlib
import os
import struct
import sys


class Mismatch(Exception):
    pass


def decode(value, name):
    """The binary key line of a base64-encoded minisign key file."""
    value = value.strip()
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


def payload(name):
    """The binary key line of the base64-encoded minisign key file in $name."""
    return decode(os.environ.get(name, ""), name)


def public_key(data, name):
    if len(data) != 42 or data[0:2] != b"Ed":
        raise Mismatch(f"{name} is not an Ed25519 minisign public key")
    return data


def key_id(public):
    """The key ID as minisign shows it (little-endian hexadecimal)."""
    return public[2:10][::-1].hex().upper()


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
    parser = argparse.ArgumentParser()
    parser.add_argument("--pinned", metavar="FILE")
    parser.add_argument("--require-variable", action="store_true")
    parser.add_argument("--public-only", action="store_true")
    args = parser.parse_args()
    variable = "DECKPILOT_UPDATER_PUBKEY"
    try:
        if args.pinned:
            try:
                with open(args.pinned, encoding="utf-8") as file:
                    pinned = file.read()
            except OSError as error:
                raise Mismatch(f"{args.pinned} cannot be read") from error
            public = public_key(decode(pinned, args.pinned), args.pinned)
            if os.environ.get(variable, "").strip():
                if public_key(payload(variable), variable) != public:
                    raise Mismatch(f"{variable} is not the public key pinned in {args.pinned}")
            elif args.require_variable:
                raise Mismatch(f"{variable} is not set")
            name = args.pinned
        else:
            public = public_key(payload(variable), variable)
            name = variable
        if args.public_only:
            print(f"public key {key_id(public)}: {name} is a well-formed Ed25519 minisign public key")
            return 0
        keynum, pk = secret_key(
            payload("TAURI_SIGNING_PRIVATE_KEY"),
            os.environ.get("TAURI_SIGNING_PRIVATE_KEY_PASSWORD", ""),
        )
        if keynum != public[2:10] or pk != public[10:42]:
            raise Mismatch(f"the signing key does not belong to {name}")
    except Mismatch as error:
        print(f"::error::{error}")
        return 1
    print(f"match: the signing key and its password belong to {name} (key {key_id(public)})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
