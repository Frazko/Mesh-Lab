# Material de terceros

Apache-2.0 se aplica al trabajo original de Mesh Lab. No sustituye las licencias
de sus dependencias, herramientas ni vectores de conformidad externos.

## Material incluido en el repositorio

| Material | Procedencia y condiciones |
|---|---|
| Wrapper Gradle (`app/android/gradlew`, `gradlew.bat` y `gradle/wrapper/gradle-wrapper.jar`) | Gradle, bajo [Apache-2.0](https://github.com/gradle/gradle/blob/master/LICENSE). Se conserva una copia de Apache-2.0 en la raíz de esta distribución. |
| Vector Noise XX (`vectors/session/noise-xx-cacophony.json`) | Extraído sin cambios de los [vectores de Snow 0.10.0](https://docs.rs/crate/snow/0.10.0/source/tests/vectors/cacophony.txt); Snow declara Apache-2.0 OR MIT. La colección original es [Cacophony](https://github.com/haskell-cryptography/cacophony), cuyo [LICENSE](https://github.com/haskell-cryptography/cacophony/blob/master/LICENSE) es Unlicense. Véase también el README del directorio del vector. |
| Vector HPKE (`vectors/crypto/hpke-rfc9180.json`) | Procede de los [vectores del proyecto CFRG HPKE](https://github.com/cfrg/draft-irtf-cfrg-hpke/blob/master/test-vectors.json). Sus [condiciones de contribución](https://github.com/cfrg/draft-irtf-cfrg-hpke/blob/master/CONTRIBUTING.md) remiten a BCP 78/79 y las disposiciones del IETF Trust, incluidas las relativas a componentes de código. No se reivindica autoría original de esos datos. |

Los valores de claves de esos vectores son fixtures públicas para pruebas, no
credenciales de instalaciones ni material de producción.

## Dependencias resueltas durante la compilación

Las versiones se fijan en `Cargo.lock` y en los `pubspec.lock` de cada paquete.
Rust, Flutter/Dart y los componentes nativos conservan los avisos de sus
respectivos titulares. SQLCipher y su proveedor criptográfico, entre otros,
se incorporan durante la construcción nativa y no se relicencian como trabajo
original de Mesh Lab.

Este documento identifica procedencia; no es un inventario exhaustivo de
licencias de un binario compilado. Quien distribuya APK, IPA, frameworks u otros
binarios debe acompañarlos de los textos y avisos de las dependencias
efectivamente incluidas en ese artefacto.
