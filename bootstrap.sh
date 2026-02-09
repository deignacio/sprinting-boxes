#!/bin/bash
set -e

# bootstrap.sh - Build static OpenCV for sb-oneshot
# Supports macOS (arm64/x86_64) and Linux
# All dependencies are built from source - no system library dependencies

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
VENDOR_DIR="$SCRIPT_DIR/vendor"
OPENCV_VERSION="4.10.0"

echo "📦 Static OpenCV Bootstrap for sb-oneshot"
echo "==========================================="
echo "Version: $OPENCV_VERSION"
echo "Vendor dir: $VENDOR_DIR"
echo ""

# Parse arguments
CLEAN_BUILD=false
for arg in "$@"; do
    case $arg in
        --clean)
            CLEAN_BUILD=true
            ;;
    esac
done

if [ "$CLEAN_BUILD" = true ]; then
    echo "🧹 Cleaning previous build..."
    rm -rf "$VENDOR_DIR/opencv-build" "$VENDOR_DIR/opencv-static"
fi

mkdir -p "$VENDOR_DIR"

# Clone OpenCV if not present
if [ ! -d "$VENDOR_DIR/opencv-src" ]; then
    echo "⬇️  Cloning OpenCV $OPENCV_VERSION..."
    git clone --depth 1 --branch $OPENCV_VERSION https://github.com/opencv/opencv.git "$VENDOR_DIR/opencv-src"
else
    echo "✅ OpenCV source already present"
fi

mkdir -p "$VENDOR_DIR/opencv-build"
cd "$VENDOR_DIR/opencv-build"

# =============================================================================
# CMake Configuration
# =============================================================================
# Philosophy: Build everything from source, disable features requiring external
# dependencies, minimize the footprint while keeping what sb-oneshot needs.

COMMON_FLAGS=(
    # Build configuration
    -DCMAKE_BUILD_TYPE=Release
    -DCMAKE_INSTALL_PREFIX="$VENDOR_DIR/opencv-static"
    -DCMAKE_POLICY_VERSION_MINIMUM=3.5
    -DBUILD_SHARED_LIBS=OFF
    -DOPENCV_GENERATE_PKGCONFIG=OFF
    
    # Disable unnecessary build targets
    -DBUILD_EXAMPLES=OFF
    -DBUILD_TESTS=OFF
    -DBUILD_PERF_TESTS=OFF
    -DBUILD_DOCS=OFF
    -DBUILD_opencv_apps=OFF
    -DBUILD_opencv_python2=OFF
    -DBUILD_opencv_python3=OFF
    -DBUILD_opencv_java=OFF
    
    # ==========================================================================
    # Image codecs - BUILD FROM BUNDLED SOURCES (no system dependencies)
    # ==========================================================================
    -DBUILD_PNG=ON
    -DBUILD_JPEG=ON
    -DBUILD_TIFF=ON
    -DBUILD_WEBP=ON
    -DBUILD_OPENJPEG=ON
    -DBUILD_ZLIB=ON
    
    # Disable external library detection for image codecs
    -DWITH_PNG=ON
    -DWITH_JPEG=ON
    -DWITH_TIFF=ON
    -DWITH_WEBP=ON
    -DWITH_OPENJPEG=ON
    
    # ==========================================================================
    # DISABLE features requiring external dependencies we don't want to bundle
    # ==========================================================================
    # OpenEXR - HDR image format, requires external libImath/OpenEXR
    -DWITH_OPENEXR=OFF
    
    # OrbbecSDK - 3D depth camera support
    -DWITH_OBSENSOR=OFF
    
    # Intel TBB/IPP - performance libraries (optional, adds complexity)
    -DWITH_TBB=OFF
    -DWITH_IPP=OFF
    -DWITH_ITT=OFF
    
    # Video/multimedia backends we don't need
    -DWITH_FFMPEG=OFF
    -DWITH_GSTREAMER=OFF
    
    # GUI backends we don't need
    -DWITH_GTK=OFF
    -DWITH_QT=OFF
    -DWITH_VTK=OFF
    
    # Other optional features we don't need
    -DWITH_PROTOBUF=OFF
    -DWITH_CUDA=OFF
    -DWITH_OPENCL=OFF
    -DWITH_1394=OFF
    -DWITH_ARAVIS=OFF
    -DWITH_EIGEN=OFF
    -DWITH_LAPACK=OFF
    -DWITH_QUIRC=OFF
)

# Platform-specific flags
if [[ "$OSTYPE" == "darwin"* ]]; then
    echo "🍎 Detected macOS"
    ARCH=$(uname -m)
    PLATFORM_FLAGS=(
        -DCMAKE_OSX_ARCHITECTURES=$ARCH
        -DCMAKE_OSX_DEPLOYMENT_TARGET=11.0
        # AVFoundation for camera capture on macOS
        -DWITH_AVFOUNDATION=ON
    )
    NPROC=$(sysctl -n hw.ncpu)
elif [[ "$OSTYPE" == "linux-gnu"* ]]; then
    echo "🐧 Detected Linux"
    PLATFORM_FLAGS=(
        -DWITH_V4L=ON
        -DWITH_OPENGL=OFF
    )
    NPROC=$(nproc)
else
    echo "❌ Unsupported platform: $OSTYPE"
    exit 1
fi

echo "🛠️  Configuring CMake..."
echo "   Using $NPROC cores for build"
cmake ../opencv-src "${COMMON_FLAGS[@]}" "${PLATFORM_FLAGS[@]}"

echo ""
echo "🔨 Building OpenCV (this may take 15-30 minutes)..."
make -j$NPROC

echo "📥 Installing to $VENDOR_DIR/opencv-static..."
make install

# =============================================================================
# Post-install: Create symlinks for 3rdparty libraries
# =============================================================================
# The 3rdparty libs have names like "liblibpng.a" but linker expects "libpng.a"
echo "🔗 Creating library symlinks..."
THIRDPARTY_DIR="$VENDOR_DIR/opencv-static/lib/opencv4/3rdparty"

if [ -d "$THIRDPARTY_DIR" ]; then
    cd "$THIRDPARTY_DIR"
    for f in liblib*.a; do
        if [ -f "$f" ]; then
            # Extract the base name (liblibpng.a -> libpng.a)
            base=$(echo "$f" | sed 's/^lib//')
            if [ ! -e "$base" ]; then
                ln -sf "$f" "$base"
                echo "   $f -> $base"
            fi
        fi
    done
fi

# =============================================================================
# Generate env.sh
# =============================================================================
echo "📝 Generating env.sh..."

# Determine which OpenCV modules were built
OPENCV_LIBS=""
for lib in "$VENDOR_DIR/opencv-static/lib"/libopencv_*.a; do
    if [ -f "$lib" ]; then
        name=$(basename "$lib" .a | sed 's/^lib//')
        if [ -n "$OPENCV_LIBS" ]; then
            OPENCV_LIBS="$OPENCV_LIBS,$name"
        else
            OPENCV_LIBS="$name"
        fi
    fi
done

# Determine which 3rdparty libraries were built
THIRDPARTY_LIBS=""
if [ -d "$THIRDPARTY_DIR" ]; then
    for lib in "$THIRDPARTY_DIR"/lib*.a; do
        if [ -f "$lib" ]; then
            name=$(basename "$lib" .a | sed 's/^lib//')
            # Skip the "libXXX" duplicates, only keep base names
            if [[ ! "$name" =~ ^lib ]]; then
                if [ -n "$THIRDPARTY_LIBS" ]; then
                    THIRDPARTY_LIBS="$THIRDPARTY_LIBS,$name"
                else
                    THIRDPARTY_LIBS="$name"
                fi
            fi
        fi
    done
fi

# Platform-specific frameworks
if [[ "$OSTYPE" == "darwin"* ]]; then
    FRAMEWORKS=",framework=Accelerate,framework=AVFoundation,framework=CoreMedia,framework=CoreVideo,framework=CoreGraphics,framework=CoreFoundation"
else
    FRAMEWORKS=""
fi

cat > "$SCRIPT_DIR/env.sh" <<ENVEOF
# Source this file before building: source env.sh
# Generated by bootstrap.sh on $(date)

ENV_SCRIPT_DIR="\$(cd "\$(dirname "\${BASH_SOURCE[0]}")" && pwd)"

# Library and include paths
export OPENCV_LINK_PATHS="\$ENV_SCRIPT_DIR/vendor/opencv-static/lib,\$ENV_SCRIPT_DIR/vendor/opencv-static/lib/opencv4/3rdparty"
export OPENCV_INCLUDE_PATHS="\$ENV_SCRIPT_DIR/vendor/opencv-static/include/opencv4"

# Disable pkg-config to prevent using system OpenCV (e.g., Homebrew)
# This forces opencv-rs to use only the environment variables above
export OPENCV4_NO_PKG_CONFIG=1

# Path to libclang for binding generation (macOS specific)
if [[ "$OSTYPE" == "darwin"* ]]; then
    export LIBCLANG_PATH="/Library/Developer/CommandLineTools/usr/lib"
    export DYLD_LIBRARY_PATH="/Library/Developer/CommandLineTools/usr/lib"
fi

# All libraries to link (order matters for static linking)
# OpenCV modules, then 3rdparty libs, then platform frameworks
export OPENCV_LINK_LIBS="$OPENCV_LIBS,$THIRDPARTY_LIBS$FRAMEWORKS"
ENVEOF

echo ""
echo "✅ Bootstrap complete!"
echo ""
echo "Libraries built:"
echo "   OpenCV: $OPENCV_LIBS"
echo "   3rdparty: $THIRDPARTY_LIBS"
echo ""
echo "To build sb-oneshot with static OpenCV:"
echo "  source env.sh"
echo "  cargo build --release"
