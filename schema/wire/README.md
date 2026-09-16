# Wire scope
A003 in `architecture-spec.html` is normative. F0's `schema/api/foundation.cddl`
is a local native/Rust diagnostic contract, **not** the mesh wire schema.
Canonical packet/object schemas and crypto vectors are an F1 prerequisite;
no radio, discovery, group, plaintext transport or wire compatibility is enabled by F0.
