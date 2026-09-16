/// Product-neutral Flutter API for the Mesh field transport.
///
/// Applications consume session, group and radio state from here. Platform
/// channels, IP addresses, radio credentials and encrypted frames remain in
/// `mesh_host` and never enter a product UI.
library;

export 'src/field_mesh_client.dart';
export 'src/field_mesh_models.dart';
