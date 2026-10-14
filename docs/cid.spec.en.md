# Content IDs (CIDs)

- **Date:** 2026-10-01
- **Status:** stable
- **Editors:** [Robin Berjon](https://berjon.com/) <[robin@berjon.com](mailto:robin@berjon.com)>; [Juan Caballero](https://bumblefudge.com/) <[bumblefudge@learningproof.xyz](mailto:bumblefudge@learningproof.xyz)>
- **Issues:** [list](https://tangled.org/dasl.ing/dasl.ing/issues), [new](https://tangled.org/dasl.ing/dasl.ing/issues/new)

## Abstract

DASL CIDs are a simple structured identifier format for content addressing. They encapsulate a hash with enough metadata to be extensible (to add new hash types in the future) and to indicate whether they are pointing to raw bytes or to structured data.

## Introduction

DASL CIDs are a simple structured identifier format for content addressing. They encapsulate a hash with enough metadata to be extensible (to add new hash types in the future) and to indicate whether they are pointing to raw bytes or to structured data. If you're simply using DASL CIDs as identifiers, you can almost certainly just use the string as an opaque ID and worry no further.

A DASL CID can be represented as a string or as an array of bytes. If you wish to understand the internals of a CID, it has the following structure:

1. A `b` prefix (only in string form). This is an extensibility point for future CID encodings other than the current base32 to be supported. (Currently this is the only one.)
2. A version number, which is currently always 1.
3. A content codec, which is a flag indicating whether it is pointing to structured or raw data.
4. A hash type, that is always SHA-256 ([[sha256](#sha256)]).
5. A hash size, that is always 32 (`0x20`).
6. A digest, which is the hash of the content being identified.

## Parsing CIDs

### Parse a string-encoded CID

Use the following steps to **parse a string-encoded CID**, i.e. translate it to a bytestring:

1. Accept a string _CID_.
2. Remove the first character from _CID_ and store it in _prefix_.
3. If _prefix_ is not equal to `b`, throw an error.
4. Decode the rest of _CID_ using [the base32 algorithm from RFC4648](https://datatracker.ietf.org/doc/html/rfc4648#section-6) with a lowercase alphabet and store the result in _CID bytes_ ([[rfc4648](#rfc4648)]).
5. This results in _CID bytes_, which can be used to [decode a CID](#decode-a-cid).

### Decode a CID

Use the following steps to **decode a CID**:

1. Accept an array of bytes _CID bytes_.
2. Remove the first byte in _CID bytes_ and store it in _version_.
3. If _version_ is not equal to `1`, throw an error.
4. Remove the next byte in _CID bytes_ and store it in _codec_.
5. If _codec_ is not equal to `0x55` (raw) or `0x71` (DRISL), throw an error ([[drisl](#drisl)]).
6. Remove the next byte in _CID bytes_ and store it in _hash type_.
7. If _hash type_ is not equal to `0x12` (SHA-256), throw an error ([[sha256](#sha256)]).
8. Read one byte from _CID bytes_ and store it in _hash size_. If _hash size_ is any value other than 32 (`0x20`) , throw an error.
9. Read 32 bytes from _CID bytes_ and store them in _digest_. If there were fewer than 32 bytes left in _CID bytes_, throw an error.
10. Return _version_, _codec_, _hash type_, _hash size_, and _digest_.

## Relationship to IPFS

You don't need to understand IPFS in order to use DASL. This section is for informational purposes only.

DASL CIDs are a strict subset of [IPFS CIDs](https://docs.ipfs.tech/concepts/content-addressing/) with the following properties:

- Only modern CIDv1 CIDs are used, not legacy CIDv0.
- Only the lowercase base32 multibase encoding (the `b` prefix) is used for human-readable (and subdomain-usable) string encoding.
- Only the `raw` binary multicodec (0x55) and `dag-cbor` multicodec (0x71), with the latter used only for [[drisl](#drisl)]-conformant DAGs of CBOR objects.
- Only SHA-256 (0x12) for the hash function .
- The CID isn't the boss of anyone, but the expectation is that, regardless of size, resources _should not_ be "chunked" into a DAG or Merkle tree (as historically done with UnixFS canonicalization in IPFS systems) but rather hashed in their entirety and content-addressed directly. That being said, a DASL CID can point to a piece of [[drisl](#drisl)] metadata that describes this kind of chunking, if needed. (A separate specification may be added for that.)
- This set of options has the added advantage that all the aforementioned single-byte prefixes require no additional varint processing or byte-fiddling.

## References

### [drisl]

Robin Berjon & Juan Caballero. [DRISL — Deterministic Representation for Interoperable Structures & Links](https://dasl.ing/drisl.html). 2026-10-01. URL: <https://dasl.ing/drisl.html>

### [rfc4648]

S. Josefsson. [The Base16, Base32, and Base64 Data Encodings](https://www.rfc-editor.org/rfc/rfc4648). October 2006. URL: <https://www.rfc-editor.org/rfc/rfc4648>

### [sha256]

National Institute of Standards and Technology, Secure Hash Algorithm. NIST FIPS 180-2. August 2002.
