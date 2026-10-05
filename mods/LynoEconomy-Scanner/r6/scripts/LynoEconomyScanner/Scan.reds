// Economy scanner: dumps every item record with its price chain and the price the game computes for it.
// Run from the CET console in a loaded save: Game.LynoEconomyScan()
// Output: r6/storages/LynoEconomy/items.tsv and rewards.tsv (under MO2 they land in overwrite/).
import RedFileSystem.*

// RedFileSystem hands a storage out once per name, so it lives in a service that outlives game sessions.
public class LynoEconomyService extends ScriptableService {
  private let storage: ref<FileSystemStorage>;

  private cb func OnLoad() {
    this.storage = FileSystem.GetStorage("LynoEconomy");
  }

  public func Storage() -> ref<FileSystemStorage> = this.storage;
}

public static exec func LynoEconomyScan(gi: GameInstance) -> Void {
  let service = GameInstance.GetScriptableServiceContainer().GetService(n"LynoEconomyService") as LynoEconomyService;
  let player = GetPlayer(gi);
  if !IsDefined(service) || !IsDefined(service.Storage()) || !IsDefined(player) {
    LogChannel(n"DEBUG", "[LynoEconomy] scan needs RedFileSystem, Codeware and a loaded save");
    return;
  }

  let lines: array<String>;
  ArrayPush(lines, "id\tclass\ttype\tcategory\tquality\ttags\tbuyPrice\tsellPrice\tbuy\tsell\tresult");
  // GetRecords may or may not include subclasses; the exact class check below keeps each record once either way.
  let classes = [n"gamedataItem_Record", n"gamedataWeaponItem_Record", n"gamedataGadget_Record",
    n"gamedataClothing_Record", n"gamedataConsumableItem_Record", n"gamedataItemRecipe_Record", n"gamedataRecipeItem_Record"];
  for cls in classes {
    for record in TweakDBInterface.GetRecords(cls) {
      if Equals(record.GetClassName(), cls) {
        ArrayPush(lines, LynoEconomyRow(gi, player, record as Item_Record));
      }
    }
  }

  let ok = service.Storage().GetFile("items.tsv").WriteLines(lines);
  LogChannel(n"DEBUG", s"[LynoEconomy] \(ArraySize(lines) - 1) items, written: \(ok)");

  // Money rewards: quests, gigs, NCPD. The amount is the quantity chain, same notation as prices.
  let rewards: array<String>;
  ArrayPush(rewards, "id\tmoney");
  for record in TweakDBInterface.GetRecords(n"gamedataRewardBase_Record") {
    let packages: array<wref<CurrencyReward_Record>>;
    (record as RewardBase_Record).CurrencyPackage(packages);
    for package in packages {
      if IsDefined(package.Currency()) && package.Currency().GetID() == t"Items.money" {
        let quantity: array<wref<StatModifier_Record>>;
        package.QuantityModifiers(quantity);
        ArrayPush(rewards, TDBID.ToStringDEBUG(record.GetID()) + "\t" + LynoEconomyChain(quantity));
      }
    }
  }
  ok = service.Storage().GetFile("rewards.tsv").WriteLines(rewards);
  LogChannel(n"DEBUG", s"[LynoEconomy] \(ArraySize(rewards) - 1) money rewards, written: \(ok)");
}

// buy/sell are the game's own numbers for an unowned item with the player as vendor (multiplier 1.0):
// the same call Virtual Atelier uses for stores without fixed prices. Quality and level of a real drop may differ.
func LynoEconomyRow(gi: GameInstance, player: ref<GameObject>, item: ref<Item_Record>) -> String {
  let id = ItemID.FromTDBID(item.GetID());
  let tags = "";
  for tag in item.Tags() {
    tags += NameToString(tag) + ",";
  }
  let buyChain: array<wref<StatModifier_Record>>;
  let sellChain: array<wref<StatModifier_Record>>;
  item.BuyPrice(buyChain);
  item.SellPrice(sellChain);
  let recipe = item as ItemRecipe_Record;
  let result = IsDefined(recipe) && IsDefined(recipe.CraftingResult()) ? LynoEconomyName(recipe.CraftingResult().Item()) : "";
  return TDBID.ToStringDEBUG(item.GetID()) + "\t" + NameToString(item.GetClassName())
    + "\t" + LynoEconomyName(item.ItemType()) + "\t" + LynoEconomyName(item.ItemCategory()) + "\t" + LynoEconomyName(item.Quality())
    + "\t" + tags + "\t" + LynoEconomyChain(buyChain) + "\t" + LynoEconomyChain(sellChain)
    + "\t" + ToString(RPGManager.CalculateBuyPrice(gi, player, id, 1.0)) + "\t" + ToString(RPGManager.CalculateSellPrice(gi, player, id)) + "\t" + result;
}

func LynoEconomyName(record: wref<TweakDBRecord>) -> String {
  return IsDefined(record) ? TDBID.ToStringDEBUG(record.GetID()) : "";
}

// One modifier per entry: record id, then what it does (constant value, or the stat a curve/combined modifier reads).
func LynoEconomyChain(chain: array<wref<StatModifier_Record>>) -> String {
  let out = "";
  for mod in chain {
    out += TDBID.ToStringDEBUG(mod.GetID()) + "=" + NameToString(mod.ModifierType()) + ":";
    if IsDefined(mod as ConstantStatModifier_Record) {
      out += ToString((mod as ConstantStatModifier_Record).Value());
    } else if IsDefined(mod as CombinedStatModifier_Record) {
      let c = mod as CombinedStatModifier_Record;
      out += "combined(" + NameToString(c.RefObject()) + "." + LynoEconomyName(c.RefStat()) + NameToString(c.OpSymbol()) + ToString(c.Value()) + ")";
    } else if IsDefined(mod as CurveStatModifier_Record) {
      let c = mod as CurveStatModifier_Record;
      out += "curve(" + c.Id() + "/" + c.Column() + " of " + NameToString(c.RefObject()) + "." + LynoEconomyName(c.RefStat()) + ")";
    } else if IsDefined(mod as RandomStatModifier_Record) {
      let r = mod as RandomStatModifier_Record;
      out += "random(" + ToString(r.Min()) + ".." + ToString(r.Max()) + ")";
    } else {
      out += NameToString(mod.GetClassName());
    }
    out += ";";
  }
  return out;
}
