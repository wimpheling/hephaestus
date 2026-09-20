(()=>{
  const fragment=location.hash.slice(1);
  const initialSearch=location.search;
  history.replaceState(null,"",location.pathname+initialSearch);
  const initialParams=new URLSearchParams(initialSearch);
  if([...initialParams.keys()].some(key=>key!=="heph_theme")){document.body.textContent="This UI link is invalid or expired.";return;}
  const themeValues=initialParams.getAll("heph_theme");
  if(themeValues.length>1||(themeValues.length===1&&!/^(?:light|dark)$/.test(themeValues[0]))){document.body.textContent="This UI link is invalid or expired.";return;}
  const bootstrapQuery=themeValues.length===1?`?heph_theme=${themeValues[0]}`:"";
  if(!/^[A-Za-z0-9_-]{43}$/.test(fragment)){document.body.textContent="This UI link is invalid or expired.";return;}
  fetch("/_heph/bootstrap"+bootstrapQuery,{method:"POST",headers:{"Content-Type":"text/plain"},body:fragment,credentials:"same-origin"})
    .then(response=>{if(!response.ok)throw new Error("bootstrap");return response.json();})
    .then(value=>{
      const expectedParent=document.querySelector('meta[name="heph-platform-origin"]')?.content;
      if(!value||typeof value.route!=="string"||!value.route.startsWith("/")||value.route.startsWith("//")){throw new Error("route");}
      const rawPath=value.route.split("?",1)[0];
      if(!/^[A-Za-z0-9._~/-]+$/.test(rawPath)||rawPath.includes("//")||rawPath.includes("\\")||rawPath.split("/").some(segment=>segment==="."||segment==="..")){throw new Error("route");}
      const destination=new URL(value.route,location.origin);
      if(destination.origin!==location.origin||[...destination.searchParams.keys()].some(key=>key!=="heph_theme")){throw new Error("route");}
      const routeThemes=destination.searchParams.getAll("heph_theme");
      if(routeThemes.length>1||(routeThemes.length===1&&!/^(?:light|dark)$/.test(routeThemes[0]))){throw new Error("route");}
      if(typeof value.theme_origin!=="string"||value.theme_origin!==expectedParent||!/^https:\/\/[^/@?#]+(?::[0-9]+)?$/.test(value.theme_origin)){throw new Error("origin");}
      destination.searchParams.set("heph_theme_origin",value.theme_origin);
      location.replace(destination.pathname+"?"+destination.searchParams.toString());
    }).catch(()=>{document.body.textContent="This UI link is invalid or expired.";});
})();
