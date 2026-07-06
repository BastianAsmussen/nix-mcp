{
  description = "Offline nix_mcp test fixture: no inputs, so locking needs no network";

  outputs =
    { self }:
    {
      greeting = "hello from the trivial fixture flake";
      fixtureVersion = 1;
    };
}
