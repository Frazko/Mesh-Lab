CREATE TABLE meta (singleton INTEGER PRIMARY KEY CHECK(singleton=1), origin BLOB NOT NULL CHECK(length(origin)=32), next_sequence INTEGER NOT NULL CHECK(next_sequence>0), schema_hash BLOB NOT NULL CHECK(length(schema_hash)=32));
CREATE TABLE operations (id BLOB PRIMARY KEY CHECK(length(id)=16), command_hash BLOB NOT NULL CHECK(length(command_hash)=32), sequence INTEGER NOT NULL UNIQUE CHECK(sequence>0), object_id BLOB UNIQUE);
CREATE TABLE objects (id BLOB PRIMARY KEY CHECK(length(id)=32), origin BLOB NOT NULL CHECK(length(origin)=32), sequence INTEGER NOT NULL, manifest BLOB NOT NULL, reserved_bytes INTEGER NOT NULL CHECK(reserved_bytes>0), complete INTEGER NOT NULL DEFAULT 0 CHECK(complete IN (0,1)), UNIQUE(origin,sequence));
CREATE TABLE chunks (object_id BLOB NOT NULL REFERENCES objects(id), idx INTEGER NOT NULL CHECK(idx>=0 AND idx<64), payload BLOB NOT NULL CHECK(length(payload)>0 AND length(payload)<=1024), PRIMARY KEY(object_id,idx));
CREATE TABLE outbox (object_id BLOB PRIMARY KEY REFERENCES objects(id));
CREATE TABLE local_deliveries (object_id BLOB PRIMARY KEY REFERENCES objects(id), committed_at INTEGER NOT NULL CHECK(committed_at>=0));
PRAGMA user_version=1;
