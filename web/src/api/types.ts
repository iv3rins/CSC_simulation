export interface Player {id:number;name:string;age:number;role:string;retired:boolean;career:Record<string,unknown>|null;pro:{morale:number};fatigue:number}
export interface TeamRef {id:number;name:string;vrs_ranking:number}
export interface Team extends TeamRef {vrs_value:number;chemistry:{cohesion:number};roster_ids:number[]}
export interface Season {year:number;team_name:string|null;events_played:number;maps_played:number;rating:number;wins:number;honours:string[]}
export interface View {date:string;sim_year:number;world:{players:Player[];team:Team|null;teams:TeamRef[]};archive:Season[];next_milestone:{event_name:string;date:string;days_until:number}|null;season:{maps_all:number;all_rating:number;wins:number}|null;team_count:number;player_count:number}
export interface Choice {id:string;label:string;impact_certain:string;impact_possible:string}
export interface Scene {instance_id:string;scene_id:string;title:string;chapter_title:string;date:string;location:string;actor_role:string;actor_name:string|null;paragraphs:string[];choices:Choice[]}
export interface Outcome {immediate:string[];tracking:string[]}
export interface Receipt {request_id:string;scene_instance_id:string;choice_id:string;status:string;outcome:Outcome}
export interface Career {game_id:number;generation:string;story:boolean;status:string;capabilities:{advance_world:boolean};scene:Scene|null;progress:Record<string,unknown>;view:View;reason?:string}
export interface Fixture {fixture_id:number;team_a:number;team_b:number;best_of:number;status:string;result?:{score?:{team_a_score:number;team_b_score:number}}}
export interface Tournament {name:string;date:string;tier:string}
export interface Calendar {year:number;player_events:{event:Tournament;status:string;fixtures:Fixture[]}[];plan:Tournament[]}
export interface EventDetail {event_name:string;date:string;status:string;teams:TeamRef[];fixtures:Fixture[];emptiness_reason:string|null}
export interface WorldStories {stories:{team_id:number;team_name:string;headline:string;body:string;vrs_ranking:number}[];rivalries:{team_a:string;team_b:string;headline:string}[]}
export interface News {items:{date:string;event_name:string;winner:string;loser:string;score:string}[]}

