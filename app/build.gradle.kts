import java.util.Properties

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

// Signierschlüssel liegt nie im Repo: Pfad und Passwörter aus ~/.gradle/gradle.properties
// (NRZMUSIC_KEYSTORE, NRZMUSIC_KEYSTORE_PASSWORD, NRZMUSIC_KEY_ALIAS, NRZMUSIC_KEY_PASSWORD)
// oder gleichnamigen Umgebungsvariablen.
fun secret(name: String): String? =
    (project.findProperty(name) as String?) ?: System.getenv(name)

// Version folgt dem Server (Cargo.toml im Hauptordner)
val serverVersion: String = run {
    val cargo = rootProject.file("../Cargo.toml").readText()
    Regex("""(?m)^version\s*=\s*"([^"]+)"""").find(cargo)?.groupValues?.get(1) ?: "0.0.0"
}
val versionCodeFromName: Int = serverVersion.split(".").map { it.toIntOrNull() ?: 0 }
    .let { (it.getOrElse(0) { 0 } * 10000) + (it.getOrElse(1) { 0 } * 100) + it.getOrElse(2) { 0 } }

android {
    namespace = "ch.zweers.nrzmusic"
    compileSdk = 35

    defaultConfig {
        applicationId = "ch.zweers.nrzmusic"
        minSdk = 26
        targetSdk = 35
        versionCode = versionCodeFromName
        versionName = serverVersion
    }

    signingConfigs {
        create("release") {
            val ks = secret("NRZMUSIC_KEYSTORE")
            if (ks != null) {
                storeFile = file(ks)
                storePassword = secret("NRZMUSIC_KEYSTORE_PASSWORD")
                keyAlias = secret("NRZMUSIC_KEY_ALIAS") ?: "nrzmusic"
                keyPassword = secret("NRZMUSIC_KEY_PASSWORD") ?: secret("NRZMUSIC_KEYSTORE_PASSWORD")
            }
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            if (secret("NRZMUSIC_KEYSTORE") != null) {
                signingConfig = signingConfigs.getByName("release")
            }
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions {
        jvmTarget = "17"
    }
    buildFeatures {
        buildConfig = true
    }
    lint {
        // media3-ui (PlayerView-Einstellungen) ist als "unstable" markiert – kein Grund zum Abbruch
        abortOnError = false
        checkReleaseBuilds = false
    }
}

dependencies {
    implementation("androidx.core:core-ktx:1.15.0")
    implementation("androidx.media3:media3-exoplayer:1.5.1")
    implementation("androidx.media3:media3-session:1.5.1")
    implementation("androidx.media3:media3-ui:1.5.1")
}
