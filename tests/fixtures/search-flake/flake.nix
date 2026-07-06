# Fixture for the pinned nix_search backend: fakes exactly the two nixpkgs
# surfaces the generated expression touches (`legacyPackages.<system>` and
# `lib.nixosSystem`), so the worker tests run offline against a stable,
# hand-shaped dataset. The `broken` package and the throwing `_module` node
# exist to prove the search degrades instead of failing.
{
  description = "nix_mcp search fixture";

  outputs = _: let
    systems = [ "x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin" ];
    packages = {
      hello = {
        version = "2.12";
        meta.description = "fixture hello";
      };
      ripgrep = {
        version = "99.0";
        meta.description = "fixture ripgrep";
      };
      ripgrep-all = {
        version = "1.0";
        meta.description = "fixture ripgrep-all";
      };
      broken = throw "fixture broken package";
    };
  in {
    legacyPackages = builtins.listToAttrs (map (s: {
      name = s;
      value = packages;
    }) systems);

    lib.nixosSystem = _: {
      options = {
        _module = throw "the walker must skip underscore-prefixed names";
        networking.hostName = {
          _type = "option";
          type.description = "string";
          description = "The host name.";
          default = "nixos";
        };
        services.foo = {
          enable = {
            _type = "option";
            type.description = "boolean";
            description = "Whether to enable foo.";
            defaultText.text = "false";
            example = true;
          };
          port = {
            _type = "option";
            type.description = "signed integer";
            description = "Port for foo.";
            default = 8080;
          };
        };
      };
    };
  };
}
