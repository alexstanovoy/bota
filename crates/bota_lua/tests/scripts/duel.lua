function Think()
    local bot = GetBot()
    if bot == nil or not bot:IsAlive() then return end
    assert(bot:GetUnitName() == 'npc_dota_hero_nevermore')
    assert(bot:GetMaxHealth() >= bot:GetHealth())
    assert(GetTeam() == TEAM_RADIANT)
    bot:Action_AttackMove(Vector(0, 0, 0))
end
