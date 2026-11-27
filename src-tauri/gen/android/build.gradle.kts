buildscript {
    repositories {
        google()
        mavenCentral()
    }
    dependencies {
        classpath("com.android.tools.build:gradle:9.3.1")
        classpath("org.jetbrains.kotlin:kotlin-gradle-plugin:2.2.10")
    }
}

allprojects {
    // Tauri's Android JSON bridge uses Jackson. Keep its modules on the same
    // supported patch line instead of the template's vulnerable 2.15.3.
    configurations.configureEach {
        resolutionStrategy.eachDependency {
            if (requested.group.startsWith("com.fasterxml.jackson")) {
                useVersion("2.18.11")
                because("OSV audit: patched JSON parser and databind vulnerabilities")
            }
        }
    }
    repositories {
        google()
        mavenCentral()
    }
}

tasks.register("clean").configure {
    delete("build")
}

