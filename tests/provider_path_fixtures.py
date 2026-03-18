def posix_npm_global_module_path(package, entrypoint, prefix="/usr/local"):
    return f"{prefix}/lib/node_modules/{package}/{entrypoint}"


def posix_brew_cellar_module_path(formula, package, entrypoint, prefix="/opt/homebrew"):
    return f"{prefix}/Cellar/{formula}/ANY_VERSION/libexec/lib/node_modules/{package}/{entrypoint}"


def windows_npm_bin_path(binary_name):
    return rf"C:\Users\test\AppData\Roaming\npm\{binary_name}.cmd"


def windows_npm_module_path(package, entrypoint):
    package_path = package.replace("/", "\\")
    entrypoint_path = entrypoint.replace("/", "\\")
    return rf"C:\Users\test\AppData\Roaming\npm\node_modules\{package_path}\{entrypoint_path}"
