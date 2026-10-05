# LYNO Economy

Economy overhaul for Cyberpunk 2077 (game 2.3x), in `mods/`. It is part of the LYNO//HARDWIRED build and is
also published on Nexus as a standalone mod. It replaces EconomyPunk: the two scale the same numbers, so never
enable both.

The idea: no per-item price lists. A TweakXL `ScriptableTweak` walks every record in TweakDB, vanilla and
modded, and prices it by rule. A new mod's content follows the rules without a patch, as long as it lives in
TweakDB.

## Files

| Path | What |
|---|---|
| `mods/LynoEconomy/r6/scripts/LynoEconomy/Prices.reds` | Cyberware and schematic prices |
| `.../Income.reds` | Quest, gig, NCPD, bounty and loot money |
| `.../Vehicles.reds` | Car prices (Autofixer, `dealerPrice`, Virtual Car Dealer buy-back) |
| `mods/LynoEconomy-CourierJobs/r6/scripts/LynoEconomyCourierJobs/CourierJobs.reds` | NC Courier Jobs payouts, separate patch mod (needs NC Courier Jobs) |
| `mods/LynoEconomy-Scanner/r6/scripts/LynoEconomyScanner/Scan.reds` | Dump tool, separate mod, not for players |
| `mods/LynoEconomy.nexus.txt` | Nexus page text, split by the editor's sections |
| `mods/LynoEconomy-Dedka/` | Dedka catalog at 1.5x. **Local only**: no permission to redistribute Dedka's files |

Requirements: RED4ext, redscript, TweakXL. The scanner also needs CET, Codeware, RedFileSystem.

## Settings (decided with the build author)

| Knob | Value | Meaning |
|---|---|---|
| `LynoEconomyCyberwareMultiplier` | 0.6 | Vanilla `Price.CyberwareMultiplier` is 0.4: implants are 1.5x dearer (1.0 = 2.5x was playtested: too much) |
| `LynoEconomyRecipeBuy` | 3.0 | Schematic buy price = 3x one crafted item (`LynoEconomy.RecipeBuy`); selling keeps vanilla `Price.Recipe` 0.7. At 0.7 schematics felt too cheap |
| `LynoEconomyWeaponBuy` / `LynoEconomyWeaponSell` | 0.75 / 0.3 | Guns on the vanilla chain: appended `LynoEconomy.WeaponBuy` / `WeaponSell` (clones of `Price.BuyMultiplier`). Selling looted guns was the money exploit |
| `LynoEconomyStoryIncome` | 0.7 | Main/side quests, gigs (`sts_`), races, fights |
| `LynoEconomyGrindIncome` | 0.4 | NCPD/cyberpsycho (`QuestRewards.ma_`), `BountyReward.*`, `QuestRewards.MinigameMoney`, loot cash and shards, NC Courier Jobs |
| `LynoEconomyVehicleMultiplier` | 1.5 | All car prices |

Out of scope on purpose: clothing (Virtual Atelier included), expenses added by mods (rent, Trauma Team, fuel,
repairs), gambling.

## How the game prices things

- `Item.buyPrice` / `Item.sellPrice` are arrays of StatModifier records multiplied together (Constant,
  Combined, Curve, Random). Vanilla cyberware: `Price.BaseCyberwarePrice`, `Price.ItemQualityMultiplier`,
  `Price.PlusTierMultiplier`, `Price.CyberwareMultiplier`, then `Price.BuyPrice_StreetCred_Discount` (buy) or
  `Price.CyberwareSellMultiplier` (sell); some implants add one inline multiplier.
- Schematic = `Price.Recipe` (0.7) x the crafted item's chain. Mods often type a fixed number instead; the rule
  rebuilds the chain from the result (skipped for clothing): buy with `LynoEconomy.RecipeBuy`, sell with `Price.Recipe`.
- Cyberware whose chain has no `Price.ItemQualityMultiplier` has a typed-in price (e.g. DigitalVixen 6 000 to
  750 000) and gets the vanilla chain. An empty chain means "not for sale": left empty.
- Money rewards: `RewardBase_Record.currencyPackage` → `CurrencyReward_Record` (`currency == Items.money`) →
  `quantityModifiers`. Constant/Random amounts are scaled in place (deduped: records are shared); curves
  (level-scaled bounties, breaches) get an appended `LynoEconomy.GrindIncome` / `StoryIncome` multiplier, the
  same way Black Chrome appends `BlackChrome.RewardMultiplier` (which must never be scaled: `LynoEconomyIsAmount`).
- Category by name prefix via `TDBID.ToStringDEBUG` (works in `OnApply`).
- Loot: `Price.MoneyShard*`, `Price.*_BaseMoney`, `Items.MoneyLootTable_inline0.dropCountMin/Max`.
- Vehicles: `VehicleOffer_Record.Price()` → `EconomicAssignment.<vehicle>.overrideValue` (Int32); modded cars
  use the `Vehicle.<vehicle>.dealerPrice` flat. Vanilla quest cars without an offer still have an
  `EconomicAssignment.<name>` that Garage and Chop Shop look up by name, so those are scaled too.
- `OnApply` runs after YAML tweaks, so records created by other mods' YAML are covered. Order between
  scriptable tweaks is undefined: anything another mod's ScriptableTweak writes must be fixed later
  (see Virtual Car Dealer below).

## Other mods in the build

| Mod | How money flows | What LYNO Economy does |
|---|---|---|
| EconomyPunk | Rewrites prices and rewards | **Conflict**, disabled |
| Black Chrome | Appends a reward multiplier from CET Lua | Left alone, stacks |
| Autofixer Customs, SDH0 cars | `Vehicle.*.dealerPrice` | Scaled 1.5x |
| Virtual Car Dealer | Buy-back from `Vehicle.*.autofixer` (its own ScriptableTweak, vanilla copies); modded cars via bundles | `LynoEconomyVehicleSync` (ScriptableSystem) sets `.autofixer` to the scaled Autofixer price at game start |
| Garage (GarageCore) | `dealerPrice` → `EconomicAssignment` → config fallback (`PricingEngine.lua`) | Follows automatically |
| Garage Chop Shop | Same lookup, `config/chopshop_prices.lua` fallback | Follows automatically |
| NCGT | Services from `.autofixer`/`.dealerPrice`; money to the player only as rebates/refunds | Follows automatically, no patch |
| NC Courier Jobs | Own redscript payouts, `CJ_RuntimeSystem.JobPayoutMultiplier` | Wrapped x0.4; its own sliders (`cj_hud.json`) stack |
| Dedka Car Dealership | Own JSON catalog `cars/ra1.json` (`price.low/high`) | Not reachable from TweakDB; scaled copy local only |
| Gambling Website (DeDeGames) | CET Lua, player's own stakes | Left alone |
| Wannabe Edgerunner, DVC consumables | Typed-in prices (4 600 to 5 400, 3 500) | Not covered yet |

## Checklist: a new mod in the build

1. Run the scanner (below) and look for the mod's items. `buyPrice` chain tells which rule covers it.
2. Items/cyberware/schematics in TweakDB: covered. Odd numbers usually mean a typed-in price on a type the
   rules skip (consumables, crafting materials).
3. Money it pays: grep its scripts for `GiveItem(.*Money`, `GiveMoney`, `AddToInventory('Items.money'`.
   Reward records are covered; script payouts need a patch: a separate `LynoEconomy-<Mod>` folder with a plain
   `import Module.Class` and `@wrapMethod(Class)` of the function that computes the amount, like
   `LynoEconomy-CourierJobs`. Annotations reject module-qualified names (`INVALID_ANN_USE`), and an `@if`-guarded
   import doesn't bring the class into scope (`UNRESOLVED_REF`), so the patch can't live inside the main mod. Income is story (0.7) or grind (0.4).
4. Vehicles: if it reads `dealerPrice`/`EconomicAssignment`/`.autofixer`, nothing to do. A JSON or Lua price
   table is out of reach.
5. Expenses (rent, fees, fuel): leave them.
6. Add a row to the table above.

## Checklist: a dependency updated

- **NC Courier Jobs**: `CJ_RuntimeSystem.JobPayoutMultiplier(jobType: String) -> Float` must still exist with
  that signature, otherwise redscript fails to compile **all** scripts. Check `CJ_Runtime.reds`.
- **Virtual Car Dealer**: still writes `Vehicle.*.autofixer` as Int32?
- **Garage / Chop Shop / NCGT**: price lookup order unchanged (`PricingEngine.lua`, `ChopShopPriceEngine.reds`,
  `shop_runtime.lua` `vehicleMarketValue`).
- **Dedka**: catalog changed → regenerate the local copy (round `low`/`high` x1.5 to 500; keep CRLF line
  endings, diff must be two lines per car).
- **Game patch**: re-check the vanilla chain record names used in `Prices.reds` and the reward prefixes.

## Verifying in game

redscript log lines (`r6/logs/redscript_rCURRENT.log`):

```
[LynoEconomy] repriced W weapons, N recipes, M cyberware
[LynoEconomy] income: X story/gig rewards, Y NCPD/bounty rewards, Z level-scaled rewards, W loot values scaled
[LynoEconomy] vehicles: N Autofixer prices, M dealer prices scaled
[LynoEconomy] vehicles: K Virtual Car Dealer prices synced
```

Scanner: CET console `Game.LynoEconomyScan()` writes `items.tsv` (id, class, type, category, quality, tags,
buyPrice chain, sellPrice chain, buy, sell, crafted result) and `rewards.tsv` (id, money chain) to
`r6/storages/LynoEconomy/` (MO2 overwrite). Notes: `buy` for unowned items is quality-neutral (~10.6 quality
factor), `sell` is 0 for unowned items; an appended multiplier shows as `<TDBID:...>` (cosmetic).

Verified values: NCPD 3900→1560, Regina 3000→2100, big race 5790→4053, Pacifica 50000→35000,
`BountyReward.bounty_fingers` 160, DVC cyberware 500000→2380 (at multiplier 0.4).
Playtest feedback: car prices good; cyberware at 1.0 too expensive (now 0.6); vanilla weapon prices felt high and selling loot guns was abusable (now buy 0.75, sell 0.3); schematics too cheap at 0.7 (now 3x the item to buy).
Not yet verified in game: cyberware at 0.6, weapon rule, schematics at 3x, loot scaling, NC Courier Jobs patch.

## Known limitations

- Amounts in quest and fixer texts are strings: they show vanilla numbers.
- Script payouts of mods without a patch are not scaled.

## Releasing on Nexus

Package without macOS metadata (archives must start with `r6/`):

```sh
cd mods/LynoEconomy && zip -qrX ../LynoEconomy-<ver>.zip r6 -x '.*' '*/.*'
cd ../LynoEconomy-CourierJobs && zip -qrX ../LynoEconomy-CourierJobs-<ver>.zip r6 -x '.*' '*/.*'
cd ../LynoEconomy-Scanner && zip -qrX ../LynoEconomy-Scanner-<ver>.zip r6 -x '.*' '*/.*'
```

Main file: `LynoEconomy`. Optional: `CourierJobs` (requires NC Courier Jobs), `Scanner`. Never upload the Dedka copy. Category Gameplay; requirements
RED4ext, redscript, TweakXL. Page text: `mods/LynoEconomy.nexus.txt` (the editor is WYSIWYG, not BBCode).
Images: flat minimalist style (black and acid yellow, one object, lots of empty space); header 1300x372 with
the subject on the right, Nexus overlays the title bottom-left.
