export class ApiError extends Error { constructor(public status:number,message:string){super(message)} }
export async function request<T>(path:string,body?:unknown):Promise<T>{
  const controller = new AbortController(); const timeout = setTimeout(()=>controller.abort(),30000);
  try {const response=await fetch(path,{method:body===undefined?'GET':'POST',headers:body===undefined?{}:{'Content-Type':'application/json'},body:body===undefined?undefined:JSON.stringify(body),signal:controller.signal});
    const data=await response.json(); if(!response.ok)throw new ApiError(response.status,data.error||data.reason||`请求未完成（${response.status}）`); return data as T;
  } catch(error){if(error instanceof ApiError)throw error;throw new ApiError(0,'无法确认服务器响应。请检查连接并重试；选择尚未被自动替换。')}finally{clearTimeout(timeout)}
}
export function gamePath(id:number,path:string){return `/games/${id}/${path}`}
