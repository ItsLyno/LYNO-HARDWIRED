// Price rules applied to every item record, mods included, so new content lands in the same economy
// as the base game instead of keeping whatever its author typed. Clothing is left alone on purpose.

// Implants are the expensive goods of Night City. The game sells them at 0.4 of their price (Price.CyberwareMultiplier,
// shared by every implant, arm weapons, Black Chrome and the cyberware repriced below); 0.6 makes them 1.5x dearer,
// the same step as cars. 1.0 (2.5x) was tried and was too much.
func LynoEconomyCyberwareMultiplier() -> Float = 0.6;

// Guns are everywhere: every fight drops a pile of them, and selling the pile was the easy money of the game.
// Buying gets a bit cheaper, selling loot a lot cheaper. Both apply to vanilla and modded guns alike.
func LynoEconomyWeaponBuy() -> Float = 0.75;
func LynoEconomyWeaponSell() -> Float = 0.3;

// A schematic is a licence to craft the item forever, so it costs more than one copy of it. The game's share
// (Price.Recipe, 0.7) felt like pocket change; selling a schematic keeps the game's share.
func LynoEconomyRecipeBuy() -> Float = 3.0;

public class LynoEconomyPrices extends ScriptableTweak {
  protected cb func OnApply() -> Void {
    TweakDBManager.SetFlat(t"Price.CyberwareMultiplier.value", LynoEconomyCyberwareMultiplier());
    TweakDBManager.UpdateRecord(t"Price.CyberwareMultiplier");

    // Before recipes: a schematic copies the weapon's chain, so it follows the new weapon price.
    LynoEconomyMultiplier(t"LynoEconomy.WeaponBuy", LynoEconomyWeaponBuy());
    LynoEconomyMultiplier(t"LynoEconomy.WeaponSell", LynoEconomyWeaponSell());
    LynoEconomyMultiplier(t"LynoEconomy.RecipeBuy", LynoEconomyRecipeBuy());
    let weapons = 0;
    for record in TweakDBInterface.GetRecords(n"gamedataWeaponItem_Record") {
      if LynoEconomyPriceWeapon(record as Item_Record) {
        weapons += 1;
      }
    }
    let recipes = 0;
    for record in TweakDBInterface.GetRecords(n"gamedataItemRecipe_Record") {
      if LynoEconomyPriceRecipe(record as ItemRecipe_Record) {
        recipes += 1;
      }
    }
    let cyberware = 0;
    for record in TweakDBInterface.GetRecords(n"gamedataItem_Record") {
      if LynoEconomyPriceCyberware(record as Item_Record) {
        cyberware += 1;
      }
    }
    LogChannel(n"DEBUG", s"[LynoEconomy] repriced \(weapons) weapons, \(recipes) recipes, \(cyberware) cyberware");
  }
}

// The base game prices a schematic as Price.Recipe (a share) times the base price of what it crafts.
// Mods price schematics on their own (fixed 23000, x6000...), so they end up 100x off: rebuild every
// schematic from the full price chain of its result, bought at LynoEconomy.RecipeBuy and sold at Price.Recipe.
func LynoEconomyPriceRecipe(recipe: ref<ItemRecipe_Record>) -> Bool {
  if !IsDefined(recipe.CraftingResult()) {
    return false;
  }
  let result = recipe.CraftingResult().Item();
  if !IsDefined(result) || LynoEconomyIsClothing(result) {
    return false;
  }
  let buy = LynoEconomyIds(result, true);
  if ArraySize(buy) == 0 {
    return false;
  }
  ArrayInsert(buy, 0, t"LynoEconomy.RecipeBuy");
  let sell = LynoEconomyIds(result, false);
  ArrayInsert(sell, 0, t"Price.Recipe");
  return LynoEconomySetPrice(recipe.GetID(), buy, sell);
}

// Cyberware whose price ignores quality is a number its author typed (DigitalVixen: 6 000 to 750 000).
// It gets the chain every base-game implant shares (they differ only by an optional per-implant multiplier),
// so it scales by quality like the rest. No chain at all means not for sale (built-in or quest implants): stays that way.
func LynoEconomyPriceCyberware(item: ref<Item_Record>) -> Bool {
  let own = LynoEconomyIds(item, true);
  if !LynoEconomyIsCyberware(item) || ArraySize(own) == 0 || ArrayContains(own, t"Price.ItemQualityMultiplier") {
    return false;
  }
  return LynoEconomySetPrice(item.GetID(),
    [t"Price.BaseCyberwarePrice", t"Price.ItemQualityMultiplier", t"Price.PlusTierMultiplier", t"Price.CyberwareMultiplier", t"Price.BuyPrice_StreetCred_Discount"],
    [t"Price.BaseCyberwarePrice", t"Price.ItemQualityMultiplier", t"Price.PlusTierMultiplier", t"Price.CyberwareMultiplier", t"Price.CyberwareSellMultiplier"]);
}

// Only guns on the game's weapon chain (BaseWeaponPrice, sold through WeaponSellMultiplier): arm cyberware is
// priced as cyberware, and the few modded weapons with typed-in prices are left as their authors set them.
func LynoEconomyPriceWeapon(item: ref<Item_Record>) -> Bool {
  let buy = LynoEconomyIds(item, true);
  let sell = LynoEconomyIds(item, false);
  if item.TagsContains(n"Cyberware") || !ArrayContains(buy, t"Price.BaseWeaponPrice")
      || !ArrayContains(sell, t"Price.WeaponSellMultiplier") || ArrayContains(buy, t"LynoEconomy.WeaponBuy") {
    return false;
  }
  ArrayPush(buy, t"LynoEconomy.WeaponBuy");
  ArrayPush(sell, t"LynoEconomy.WeaponSell");
  return LynoEconomySetPrice(item.GetID(), buy, sell);
}

// A plain price factor: a copy of the game's own Price.BuyMultiplier (constant, x1.0) with another value.
func LynoEconomyMultiplier(id: TweakDBID, k: Float) -> Void {
  if !IsDefined(TweakDBInterface.GetRecord(id)) {
    TweakDBManager.CloneRecord(id, t"Price.BuyMultiplier");
  }
  TweakDBManager.SetFlat(id + t".value", k);
  TweakDBManager.UpdateRecord(id);
}

func LynoEconomyIsCyberware(item: ref<Item_Record>) -> Bool {
  return IsDefined(item.ItemType()) && item.ItemType().GetID() == t"ItemType.Cyberware";
}

func LynoEconomyIsClothing(item: ref<Item_Record>) -> Bool {
  return IsDefined(item.ItemCategory()) && item.ItemCategory().GetID() == t"ItemCategory.Clothing";
}

func LynoEconomyIds(item: ref<Item_Record>, buy: Bool) -> array<TweakDBID> {
  let chain: array<wref<StatModifier_Record>>;
  if buy {
    item.BuyPrice(chain);
  } else {
    item.SellPrice(chain);
  }
  let ids: array<TweakDBID>;
  for mod in chain {
    ArrayPush(ids, mod.GetID());
  }
  return ids;
}

func LynoEconomySetPrice(id: TweakDBID, buy: array<TweakDBID>, sell: array<TweakDBID>) -> Bool {
  TweakDBManager.SetFlat(id + t".buyPrice", buy);
  TweakDBManager.SetFlat(id + t".sellPrice", sell);
  return TweakDBManager.UpdateRecord(id);
}
