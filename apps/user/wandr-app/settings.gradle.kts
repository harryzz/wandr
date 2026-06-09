rootProject.name = "wandr-app"

// Standalone build — consumes everything from mavenLocal. No `includeBuild`
// of the skiko fork or compose-multiplatform-core; both are published as
// klibs to `~/.m2/` and we resolve them like any external dependency.
