# JDT LS missing-inner-class patch

The locked JDT LS 1.61.0 FernFlower integration aborts decompilation of an
available outer class when an entry in its InnerClasses attribute is absent.
This occurs with partial legacy libraries. The patch skips only nonexistent
class files during the integration's recursive collection. FernFlower, JDT's
Java model, source attachments and source/bytecode mappings remain upstream.
Missing class bodies are not reconstructed.

`manifest.json` fixes the upstream revision, original source and bundle hashes,
modified source hash and complete replacement class set. The original source is
Eclipse EPL-2.0; its copyright header is retained. The modified source is shipped
in `licenses/jdt-decompiler/FernFlowerDecompiler.java` with patch metadata and the
existing EPL license. The upstream source URL is recorded in the manifest.

`scripts/package/patch-jdt-decompiler.mjs` runs only while building a package.
It uses the manifest-pinned Temurin host JDK, compiles against the original
verified distribution and modifies only fresh package staging. Temporary compiler
output is invocation-owned and removed on success or failure; download caches,
installed plugins and application bundles remain read-only. Neither temporary
output nor assembled packages may be shared between worktrees.

Because the core JDT bundle's bytes change, its Eclipse JAR signature and signed
entry digests are removed before repacking. OSGi manifest attributes are retained;
unrecognized signed manifest attributes fail packaging. The rebuilt JAR is **not
Eclipse-signed**. This does not change native JVM signatures or claim a plugin
publisher signature. Packaged metadata records compiler identity, original and
patched JAR hashes, modified source hash, signature removal and every replacement
class hash. Package verification checks these identities before distribution.

Run `node --test --test-timeout=60000 scripts/package/*.test.mjs`, build the
package, and run `scripts/verify-package.sh` for the target architecture. In the
paired Viva client, `TestJavaPluginBinaryNavigation` uses its own generated JAR,
including an outer class whose nested class is deliberately omitted, and verifies
class/method locations and navigation between decompiled views. Private customer
class files are never included in the source or public test artifacts.

Remove this patch when a pinned upstream release fixes partial inner-class
collection, after the same regression passes with its unmodified distribution.
