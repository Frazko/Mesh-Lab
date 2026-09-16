CREATE TABLE policy_state (
  singleton INTEGER PRIMARY KEY CHECK(singleton=1),
  authority BLOB NOT NULL CHECK(length(authority)=32),
  group_id BLOB NOT NULL CHECK(length(group_id)=32),
  epoch INTEGER NOT NULL CHECK(epoch>0),
  roster_digest BLOB NOT NULL CHECK(length(roster_digest)=32)
);
CREATE TABLE policy_certificates (
  idx INTEGER PRIMARY KEY CHECK(idx>=0 AND idx<10),
  certificate BLOB NOT NULL CHECK(length(certificate)>0 AND length(certificate)<=1024)
);
CREATE TABLE policy_revocations (
  serial INTEGER PRIMARY KEY CHECK(serial>0)
);
PRAGMA user_version=3;
