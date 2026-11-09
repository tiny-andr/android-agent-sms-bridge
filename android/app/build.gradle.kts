import org.jetbrains.kotlin.gradle.dsl.JvmTarget
import java.util.Properties

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

// Release signing lives outside git (keystore/release.p12 + release.properties,
// generated once by tools/gen_signing.py). A stable signature is what lets an
// installed copy take an update in place.
val releaseKeystore = rootProject.file("keystore/release.p12")
val releaseProps = rootProject.file("keystore/release.properties")
val releaseSigning = Properties().apply {
    if (releaseProps.exists()) releaseProps.inputStream().use { load(it) }
}

android {
    namespace = "com.tinyandr.smsbridge"
    compileSdk = 36

    defaultConfig {
        applicationId = "com.tinyandr.smsbridge"
        minSdk = 34
        targetSdk = 36
        versionCode = 6
        versionName = "2.0.0"
    }

    signingConfigs {
        create("release") {
            if (releaseKeystore.exists()) {
                storeFile = releaseKeystore
                storePassword = releaseSigning.getProperty("storePassword")
                keyAlias = releaseSigning.getProperty("keyAlias")
                keyPassword = releaseSigning.getProperty("keyPassword")
            }
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            // The private key never enters the repository. Losing it would mean
            // every installed app has to be uninstalled before it can upgrade,
            // so tools/gen_signing.py refuses to overwrite an existing one.
            signingConfig = signingConfigs.getByName("release")
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro"
            )
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    kotlin {
        compilerOptions {
            jvmTarget.set(JvmTarget.JVM_17)
        }
    }

    buildFeatures {
        viewBinding = true
        buildConfig = true
    }

    packaging {
        resources {
            excludes += "/META-INF/{AL2.0,LGPL2.1}"
        }
    }

    // The TLS keystore is git-ignored (it holds the private key). Without this
    // check the build would succeed and the app would crash on service start.
    tasks.register("checkTlsCertificate") {
        doFirst {
            val p12 = file("src/main/assets/server.p12")
            if (!p12.exists()) {
                throw GradleException(
                    "Missing app/src/main/assets/server.p12.\n" +
                        "Run: python tools/gen_cert.py   (or tools\\gen-cert.ps1 on Windows)"
                )
            }
        }
    }

    tasks.named("preBuild") { dependsOn("checkTlsCertificate") }

    // release.p12 is git-ignored. Fail loudly instead of silently shipping a
    // release that cannot be installed over the previous one.
    tasks.register("checkReleaseKeystore") {
        doFirst {
            if (!releaseKeystore.exists() || !releaseProps.exists()) {
                throw GradleException(
                    "Missing keystore/release.p12 or keystore/release.properties.\n" +
                        "Run: python tools/gen_signing.py"
                )
            }
        }
    }

    // preReleaseBuild is created by AGP after this block is evaluated.
    tasks.matching { it.name == "preReleaseBuild" }.configureEach {
        dependsOn("checkReleaseKeystore")
    }
}

dependencies {
    implementation("androidx.core:core-ktx:1.15.0")
    implementation("androidx.appcompat:appcompat:1.7.0")
    implementation("com.google.android.material:material:1.12.0")
    implementation("androidx.constraintlayout:constraintlayout:2.2.1")
    implementation("androidx.activity:activity-ktx:1.10.0")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.10.1")
    implementation("org.nanohttpd:nanohttpd:2.3.1")
}
