// Price rules applied to every item record, mods included, so new content lands in the same economy
// as the base game instead of keeping whatever its author typed. Clothing is left alone on purpose.
public class LynoEconomyPrices extends ScriptableTweak {
  protected cb func OnApply() -> Void {
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
    LogChannel(n"DEBUG", s"[LynoEconomy] repriced \(recipes) recipes, \(cyberware) cyberware");
  }
}

// The base game prices a schematic as Price.Recipe (a share) times the base price of what it crafts.
// Mods price schematics on their own (fixed 23000, x6000...), so they end up 100x off: rebuild every
// schematic from the full price chain of its result; the share stays the game's Price.Recipe (0.7).
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
  ArrayInsert(buy, 0, t"Price.Recipe");
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
