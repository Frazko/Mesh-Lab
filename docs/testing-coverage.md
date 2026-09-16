# Política de cobertura

Desde el 14 de septiembre de 2026, cada pull request debe mantener al menos
**90% de cobertura de líneas ejecutables nuevas** de Rust y del código Flutter
de producto. La regla se aplica al diff del pull request, no a la cobertura
total histórica.

CI genera `artifacts/coverage/rust.lcov` con `cargo llvm-cov`,
`app/coverage/lcov.info` y los reportes LCOV de cada paquete Flutter de
producto con `flutter test --coverage`. El verificador
`tools/check_new_code_coverage.py` toma solo las líneas añadidas que aparecen
en esos informes; comentarios, archivos de configuración y código eliminado no
alteran el denominador.

Si un archivo Rust o Dart de producción añadido o modificado no aparece en
ningún reporte de líneas ejecutables, el gate falla. Así una prueba que nunca
carga un archivo nuevo no puede dejarlo fuera del cálculo.

La cobertura total se mide y publica como tendencia, pero no bloquea este
incremento. Kotlin y Swift requieren sus propios reportes instrumentados antes
de entrar al mismo gate de 90%; mientras tanto deben conservar tests de
contrato, integración y builds de las dos plataformas. No se declarará su
cobertura como cumplida hasta instrumentarlos.

Para ejecutarlo localmente contra una rama base:

```sh
source "$HOME/.cargo/env"
cargo llvm-cov --workspace --locked --lcov --output-path artifacts/coverage/rust.lcov
(cd app && flutter test --coverage)
(cd packages/mesh_field_sdk && flutter test --coverage)
python3 tools/check_new_code_coverage.py \
  --base origin/main \
  --rust-lcov artifacts/coverage/rust.lcov \
  --flutter-lcov app/coverage/lcov.info \
  --flutter-lcov packages/mesh_field_sdk/coverage/lcov.info
```
