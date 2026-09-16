-- The original laboratory profile capped the persisted roster at ten members.
-- Rebuild this small table transactionally so existing encrypted stores retain
-- their certificates while the field profile can certify up to fifty phones.
ALTER TABLE policy_certificates RENAME TO policy_certificates_v3;
CREATE TABLE policy_certificates (
  idx INTEGER PRIMARY KEY CHECK(idx>=0 AND idx<50),
  certificate BLOB NOT NULL CHECK(length(certificate)>0 AND length(certificate)<=1024)
);
INSERT INTO policy_certificates SELECT idx,certificate FROM policy_certificates_v3;
DROP TABLE policy_certificates_v3;
PRAGMA user_version=4;
