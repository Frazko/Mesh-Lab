CREATE TABLE auth_announcements (object_id BLOB PRIMARY KEY REFERENCES objects(id), announcement BLOB NOT NULL CHECK(length(announcement)>0 AND length(announcement)<=8192));
CREATE TABLE auth_deliveries (object_id BLOB PRIMARY KEY REFERENCES local_deliveries(object_id), receipt BLOB NOT NULL CHECK(length(receipt)>0 AND length(receipt)<=1024));
CREATE TABLE target_receipts (object_id BLOB NOT NULL REFERENCES objects(id), actor BLOB NOT NULL CHECK(length(actor)=32), receipt BLOB NOT NULL CHECK(length(receipt)>0 AND length(receipt)<=1024), PRIMARY KEY(object_id,actor));
PRAGMA user_version=2;
