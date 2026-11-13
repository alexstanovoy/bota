# Pregame laning

The pregame movement rules were established from the installed game, build 6975.
This slice covers selected Laning in a two-player Shadow Fiend Solo Mid match:
no currently observed own living creeps, an observed living own mid tier-one tower,
and no queued commands or busy statuses excluded by the fallback dispatcher.

| Rule | Behaviour | Test |
|---|---|---|
| Think override | An existing Think function suppresses default actions even when empty or returning false. | an_invoked_laning_think_consumes_native_actions_even_when_returning_false |
| Pregame phase | Use the game clock; StartGame leaves this slice immediately. | startgame_leaves_the_pregame_slice_using_the_public_game_clock |
| Lane goal | Move 300 units behind the observed own mid T1 along the team's lane. | pregame_laning_moves_without_attacking_for_either_side |
| Arrival | Clear this controller's movement at distance at most 100. | pregame_laning_arrival_is_inclusive_at_one_hundred_units |
| Unrelated actions | Arrival without a default movement continuation issues no stop. | an_idle_controller_at_the_lane_goal_does_not_issue_a_stop |
| Current facts | Remembered creeps removed from the current view do not disable the no-creep slice. | removed_creeps_in_client_history_do_not_disable_the_no_creep_slice |
| Scope | Current own creeps retain the existing fallback outside this slice. | pregame_creeps_keep_the_unported_dynamic_front_path_on_the_existing_fallback |
| Body replacement | An explicit nonqueued script body order replaces the default movement continuation. | an_explicit_script_body_order_replaces_native_laning_controller_state |
| Discarded output | Restore the continuation when its output is discarded, including cancellation. | native_laning_cancellation_rewinds_with_a_discarded_outbox |
| Host command delivery | Issue movement once, then retry after a 30-tick lease or immediately when its target changes; cancel once on arrival. | laning_move_orders_are_lease_bounded_over_six_hundred_ticks; a_changed_movement_target_replaces_an_unexpired_lease |
| Host work budget | Prepare static geometry at initialization. Tick work charges one fuel unit per inspected observation above the measured bare call and existing dispatcher base. | native_mode_callbacks_charge_the_documented_work_above_the_bare_call |
| Bent lanes | Follow the lane's arc length, including bends. | pregame_geometry_and_its_oracle_follow_a_bent_polyline |
| Unavailable actors | A dead or respawning hero emits no laning body order; a stunned or feared hero waits. | a_dead_respawning_hero_receives_no_laning_body_order; a_stunned_hero_receives_no_laning_body_order; a_feared_hero_receives_no_laning_body_order |
| Missing front | A destroyed tower leaves this movement slice. | a_destroyed_mid_tower_leaves_the_pregame_slice |
| Return movement | A hero already past its goal moves back to it. | a_hero_already_past_the_goal_moves_back_to_it |

Server-path cases are in tests/laning.rs and tests/laning/conditions.rs. Continuation and fuel pins are in
src/team/default_fuel_tests.rs. The host uses the simulation's lane geometry and
ordinary orders; exact reference geometry, command cadence and random-stream
alignment are not claimed.

Combat, dynamic fronts, native default desire and spacing for multiple bots are
outside this slice. Other situations retain the existing policy, whose thresholds
remain labelled as reconstruction assumptions.
