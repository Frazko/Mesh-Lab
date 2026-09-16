-- One visible group action can fan out into several bounded encrypted
-- audiences. Keep the origin-side delivery evidence under a single opaque
-- logical ID so the product never reports "delivered" after one audience.
CREATE TABLE logical_messages (
  id BLOB PRIMARY KEY CHECK(length(id)=16),
  created_at INTEGER NOT NULL CHECK(created_at>=0),
  expires_at INTEGER NOT NULL CHECK(expires_at>created_at),
  target_count INTEGER NOT NULL CHECK(target_count>0 AND target_count<=49),
  audience_count INTEGER NOT NULL CHECK(audience_count>0 AND audience_count<=5)
);
CREATE TABLE logical_message_objects (
  logical_id BLOB NOT NULL REFERENCES logical_messages(id),
  object_id BLOB NOT NULL UNIQUE REFERENCES objects(id),
  target_count INTEGER NOT NULL CHECK(target_count>0 AND target_count<=10),
  PRIMARY KEY(logical_id, object_id)
);
CREATE INDEX logical_message_objects_logical ON logical_message_objects(logical_id);
PRAGMA user_version=10;
