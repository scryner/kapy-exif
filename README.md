kapy-exif
=========

This is a simple library that extracts and replaces EXIF data in an image file. It focuses on the minimal functionalities required by the Kapy application, and therefore, it might be difficult to use for general purposes.

The functionalities this library focuses on are as follows:

- Extracting raw EXIF data from an image (currently, JPEG and HEIC images are supported)
- Adding or replacing raw EXIF data for an image (with 'exiv2' feature)

## Build

This section describes how to build the crate with 'exiv2' feature.
This feature is required to manipulate(get tag, update GPS info) EXIF data.

If you don't need to manipulate EXIF data, you can build the crate without the 'exiv2' feature.
In this case, you do not need to follow the instructions below.

### Build on macOS

If you use Homebrew (https://brew.sh/), you can easily install the required packages. <br/>
After installing Homebrew, you can install the required packages and build the application by running the following command:

```shell
$ brew install pkg-config exiv2
$ cargo build --features=exiv2
```

If you are not using Homebrew, please install the required packages below and set the corresponding environment variables accordingly:

* Exiv2 library (https://exiv2.org/download.html)
  * EXIV2_INCLUDE_DIRS - list of include directories split by :
  * EXIV2_LIB_DIRS - list of lib directories split by :


### Build on Windows
#### Pre-requirements
* Exiv2 library (https://exiv2.org/download.html)
  * Provides pre-built binaries as .zip compressed files.
  * You need to set the following Windows environment variables:
    * EXIV2_INCLUDE_DIRS={YOUR_EXIV2_INCLUDE_DIR}
    * EXIV2_LIB_DIRS={YOUR_EXIV_LIB_DIR}
* clang library (https://releases.llvm.org/download.html)
  * Provides pre-built binary installers.
  * You need to set the following Windows environment variable:
    * LIBCLANG_PATH={YOUR_LLVM_BIN_DIR}

### Build
```shell
> set EXIV2_INCLUDE_DIRS={YOUR_EXIV2_INCLUDE_DIR}
> set EXIV2_LIB_DIRS={YOUR_EXIV_LIB_DIR}
> set LIBCLANG_PATH={YOUR_LLVM_BIN_DIR}
> cargo build --features=exiv2
```
