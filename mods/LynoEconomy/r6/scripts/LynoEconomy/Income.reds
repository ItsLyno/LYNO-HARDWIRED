// Money the player gets from the world, scaled down so prices mean something. Story and gigs keep most
// of their pay; grind sources (NCPD hustles, cyberpsychos, cash and shards in loot) much less.
// Money rewards of mods that use reward records scale the same way. Mods that pay from their own scripts
// (NC Courier Jobs, NCGT, Gambling...) can't be reached from TweakDB, and expenses other mods add
// (rent, Trauma Team, fuel) are left alone on purpose.
func LynoEconomyStoryIncome() -> Float = 0.7;
func LynoEconomyGrindIncome() -> Float = 0.4;

public class LynoEconomyIncome extends ScriptableTweak {
  // A modifier shared by several sources is scaled once.
  private let done: array<TweakDBID>;

  protected cb func OnApply() -> Void {
    let loot = 0;
    let grind = LynoEconomyGrindIncome();
    for id in [t"Price.MoneyShardUncommon", t"Price.MoneyShardRare", t"Price.MoneyShardEpic", t"Price.MoneyShardLegendary",
        t"Price.Resource_BaseMoney", t"Price.BaseMiniStory_BaseMoney", t"Price.CyberPsycho_BaseMoney", t"Price.FailedCrossing_BaseMoney",
        t"Price.GangWatch_BaseMoney", t"Price.HiddenStash_BaseMoney", t"Price.Outpost_BaseMoney", t"Price.StreetStory_BaseMoney",
        t"Price.EndlessOutpost_BaseMoney_Small", t"Price.EndlessOutpost_BaseMoney_Big"] {
      if LynoEconomyIsAmount(TweakDBInterface.GetRecord(id) as StatModifier_Record) {
        loot += this.ScaleModifier(id, grind);
      }
    }
    // Cash piles: an item count, not a modifier.
    loot += LynoEconomyScaleInt(t"Items.MoneyLootTable_inline0.dropCountMin", grind);
    loot += LynoEconomyScaleInt(t"Items.MoneyLootTable_inline0.dropCountMax", grind);
    TweakDBManager.UpdateRecord(t"Items.MoneyLootTable_inline0");

    let story = 0;
    let hustles = 0;
    let curves = 0;
    for record in TweakDBInterface.GetRecords(n"gamedataRewardBase_Record") {
      let reward = record as RewardBase_Record;
      let name = TDBID.ToStringDEBUG(reward.GetID());
      // ma_ = minor activities (NCPD hustles, cyberpsychos); bounties on wanted NPCs; access point breaches.
      let isGrind = StrBeginsWith(name, "QuestRewards.ma_") || StrBeginsWith(name, "BountyReward.") || Equals(name, "QuestRewards.MinigameMoney");
      let k = isGrind ? grind : LynoEconomyStoryIncome();
      let packages: array<wref<CurrencyReward_Record>>;
      reward.CurrencyPackage(packages);
      for package in packages {
        if IsDefined(package.Currency()) && package.Currency().GetID() == t"Items.money" {
          let quantity: array<wref<StatModifier_Record>>;
          package.QuantityModifiers(quantity);
          let fixed = false;
          for mod in quantity {
            if LynoEconomyIsAmount(mod) {
              fixed = true;
              let n = this.ScaleModifier(mod.GetID(), k);
              if isGrind { hustles += n; } else { story += n; }
            }
          }
          // The amount is a curve (bounties and breaches scale with the target's level): multiply the result instead,
          // the way Black Chrome adds its reward bonus.
          if !fixed {
            this.AppendMultiplier(package.GetID(), isGrind);
            curves += 1;
          }
        }
      }
    }
    LogChannel(n"DEBUG", s"[LynoEconomy] income: \(story) story/gig rewards, \(hustles) NCPD/bounty rewards, \(curves) level-scaled rewards, \(loot) loot values scaled");
  }

  private func ScaleModifier(id: TweakDBID, k: Float) -> Int32 {
    if ArrayContains(this.done, id) {
      return 0;
    }
    ArrayPush(this.done, id);
    let scaled = 0;
    for flat in [t".value", t".min", t".max"] {
      let v = TweakDBInterface.GetFlat(id + flat);
      if IsDefined(v) && Equals(VariantTypeName(v), n"Float") {
        TweakDBManager.SetFlat(id + flat, FromVariant<Float>(v) * k);
        scaled = 1;
      }
    }
    if scaled > 0 {
      TweakDBManager.UpdateRecord(id);
    }
    return scaled;
  }

  private func AppendMultiplier(package: TweakDBID, isGrind: Bool) -> Void {
    let mult = isGrind ? t"LynoEconomy.GrindIncome" : t"LynoEconomy.StoryIncome";
    if !IsDefined(TweakDBInterface.GetRecord(mult)) {
      TweakDBManager.CreateRecord(mult, n"gamedataConstantStatModifier_Record");
      TweakDBManager.SetFlat(mult + t".statType", t"BaseStats.Quantity");
      TweakDBManager.SetFlat(mult + t".modifierType", n"Multiplier");
      TweakDBManager.SetFlat(mult + t".value", isGrind ? LynoEconomyGrindIncome() : LynoEconomyStoryIncome());
      TweakDBManager.UpdateRecord(mult);
    }
    let ids: array<TweakDBID>;
    let quantity: array<wref<StatModifier_Record>>;
    (TweakDBInterface.GetRecord(package) as CurrencyReward_Record).QuantityModifiers(quantity);
    for mod in quantity {
      ArrayPush(ids, mod.GetID());
    }
    ArrayPush(ids, mult);
    TweakDBManager.SetFlat(package + t".quantityModifiers", ids);
    TweakDBManager.UpdateRecord(package);
  }
}

// Only plain numbers are sums of money: constant (.value) and random (.min/.max). Curves and combined modifiers
// also carry a .value, but it is a factor of someone else's formula (Black Chrome's reward bonus is one).
func LynoEconomyIsAmount(mod: wref<StatModifier_Record>) -> Bool {
  return IsDefined(mod as ConstantStatModifier_Record) || IsDefined(mod as RandomStatModifier_Record);
}

func LynoEconomyScaleInt(flat: TweakDBID, k: Float) -> Int32 {
  let v = TweakDBInterface.GetFlat(flat);
  if !IsDefined(v) || !Equals(VariantTypeName(v), n"Int32") {
    return 0;
  }
  return TweakDBManager.SetFlat(flat, Max(1, RoundF(Cast<Float>(FromVariant<Int32>(v)) * k))) ? 1 : 0;
}
