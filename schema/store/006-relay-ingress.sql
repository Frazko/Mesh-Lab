-- The forwarded frame names the local relay as previous_hop. Keep the actual
-- authenticated ingress neighbor separately so a restarted executor never
-- sends an object directly back to the neighbor that supplied it.
ALTER TABLE relay_metadata ADD COLUMN received_from BLOB;
UPDATE relay_metadata SET received_from=previous_hop WHERE received_from IS NULL;
PRAGMA user_version=6;
